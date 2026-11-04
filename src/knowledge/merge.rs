//! Merging one entity into another.
//!
//! A merge is the operation the whole model is built to make cheap: the loser's observations are
//! re-pointed, its evidence moves, and the loser becomes a tombstone. Nothing is edited and nothing is
//! deleted, so the decision is one pointer away from being undone.
//!
//! This is the first half of §Repair in `plan/knowledge-canonical-entities.md` — the `retire` and
//! `relink` steps. The CLI that drives a repair by hand comes later; the primitive is what both the
//! identity-conflict path and that command use.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, Set, Statement, TransactionTrait,
};

use crate::model::{
    knowledge_claims as claims, knowledge_entity as entity, knowledge_merges as merges,
    user_observed_entity as observation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Merged,
    /// Nothing to do: the same pair has already been folded. The resolution loop can reach one
    /// conflict twice — the loser stays in the run's entity list after it has been merged — so a
    /// repeat must not move anything again or record a second merge event.
    AlreadyMerged,
    /// The two entities have observations in the same capture. The linker's same-capture veto already
    /// decided that those mentions are different referents, so folding them would contradict a fact
    /// the graph already holds.
    RefusedSharedIllumination,
}

/// What a merge would do, without doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preview {
    pub observations: u64,
    pub evidence: u64,
    /// Observations the two entities have in the same capture. Nonzero means the merge is refused: the
    /// linker already decided those mentions are different referents.
    pub shared_illuminations: i64,
}

/// Why a merge happened.
///
/// The merge row is the only audit trail there is, so both halves are recorded: `method` says what
/// decided it, `source` says whose decision it was. Hardcoding either makes a repair by hand
/// indistinguishable from a resolver conflict, after the fact and permanently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// Two entities resolved to one identity: `resolve` found the same claim held twice.
    IdentityConflict,
    /// A repair — someone decided these are one thing.
    Repair,
}

impl Cause {
    fn method(self) -> &'static str {
        match self {
            Self::IdentityConflict => "identity_conflict",
            Self::Repair => "repair",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::IdentityConflict => "model",
            Self::Repair => "user",
        }
    }
}

/// Fold `loser` into `winner`. See [`merge`] for what that means and why it can refuse.
///
/// The blocker is told afterwards, not inside the transaction: projecting the winner happens within
/// it, but a rollback would leave the index holding a row that never landed. The second projection is
/// idempotent and merges are rare, so it doubles as the notification.
pub(super) async fn merge_into(
    conn: &sea_orm::DatabaseConnection,
    knowledge: &super::Knowledge,
    loser: i32,
    winner: i32,
    cause: Cause,
) -> anyhow::Result<Outcome> {
    let outcome = merge(conn, loser, winner, cause).await?;
    if outcome == Outcome::Merged {
        knowledge.update_projection(winner).await?;
        knowledge.forget(loser);
    }
    Ok(outcome)
}

/// What a merge would do. Free because [`merge_in_transaction`] and the facade both use it.
pub(super) async fn preview<C: ConnectionTrait>(
    conn: &C,
    loser: i32,
    winner: i32,
) -> anyhow::Result<Preview> {
    let observations = observation::Entity::find()
        .filter(observation::Column::KnowledgeEntityId.eq(loser))
        .count(conn)
        .await?;
    let claim_count = claims::Entity::find()
        .filter(claims::Column::KnowledgeEntityId.eq(loser))
        .count(conn)
        .await?;
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM user_observed_entities l \
             JOIN user_observed_entities w ON w.illumination_id = l.illumination_id \
             WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2",
            [loser.into(), winner.into()],
        ))
        .await?;
    let shared_illuminations = row
        .map(|row| row.try_get_by_index::<i64>(0))
        .transpose()?
        .unwrap_or(0);
    Ok(Preview {
        observations,
        evidence: claim_count,
        shared_illuminations,
    })
}

/// Fold `loser` into `winner`.
///
/// The winner should be the better-substantiated entity — in practice the one that already holds the
/// identity which caused the conflict — and it must be live. A tombstoned *loser* is a no-op; a
/// tombstoned winner is refused, because the loser's observations would be stranded on a dead row.
///
/// **The same-illumination veto applies here too.** Two entities that both appear in one *illumination*
/// cannot be one referent: one extractor writing the same string twice in one screenshot means two
/// things, and the linker refused to fold those observations for exactly that reason. A merge that
/// ignored it produced four observations on one entity, two of them from the same capture — the state
/// the veto exists to prevent. In that case the conflict is real and needs adjudication, so it is
/// reported rather than resolved.
///
/// The unit is the illumination, not the capture: re-illuminating an image is a separate illumination, and
/// its names may legitimately join the same entity. See `plan/knowledge-illumination-history.md`.
async fn merge(
    conn: &DatabaseConnection,
    loser: i32,
    winner: i32,
    cause: Cause,
) -> anyhow::Result<Outcome> {
    // Five writes that must all land: repointing observations, dropping the evidence that would
    // collide, moving the rest, recording the merge and tombstoning the loser. A crash between any
    // two of them leaves two live entities claiming one identity, which nothing can repair.
    conn.transaction(move |txn| {
        Box::pin(async move { merge_in_transaction(txn, loser, winner, cause).await })
    })
    .await
    .map_err(|error| anyhow::anyhow!("merge transaction failed: {error}"))
}

async fn merge_in_transaction<C: ConnectionTrait>(
    conn: &C,
    loser: i32,
    winner: i32,
    cause: Cause,
) -> anyhow::Result<Outcome> {
    if loser == winner {
        return Ok(Outcome::AlreadyMerged);
    }
    // Both rows in one query. The loser's tombstone is what makes a repeat a no-op; either being
    // absent is a caller error — a stale id would otherwise "merge" nothing and still record the
    // event, which is a false success.
    let rows = entity::Entity::find()
        .filter(entity::Column::Id.is_in([loser, winner]))
        .all(conn)
        .await?;
    let live = |id: i32| {
        rows.iter()
            .find(|row| row.id == id)
            .map(|row| row.merged_into_id.is_none())
    };
    anyhow::ensure!(
        live(loser).is_some(),
        "knowledge entity {loser} does not exist"
    );
    anyhow::ensure!(
        live(winner).is_some(),
        "knowledge entity {winner} does not exist"
    );
    if live(loser) == Some(false) {
        return Ok(Outcome::AlreadyMerged);
    }
    // A tombstone is a forwarding pointer, not a home: observations landing on one are stranded.
    anyhow::ensure!(
        live(winner) == Some(true),
        "knowledge entity {winner} is a tombstone; merge into its canonical entity"
    );

    if preview(conn, loser, winner).await?.shared_illuminations > 0 {
        return Ok(Outcome::RefusedSharedIllumination);
    }

    // Observations follow the winner. This is the only column a merge touches on them.
    observation::Entity::update_many()
        .col_expr(
            observation::Column::KnowledgeEntityId,
            Expr::value(Some(winner)),
        )
        .filter(observation::Column::KnowledgeEntityId.eq(loser))
        .exec(conn)
        .await?;

    // Evidence that duplicates the winner's would violate the identity index on the way over, so it
    // is dropped first. It is the same fact stated twice, and the winner's copy is the one kept.
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM knowledge_claims l USING knowledge_claims w \
         WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2 \
           AND w.retracted_at IS NULL \
           AND l.knowledge_identity_id = w.knowledge_identity_id",
        [loser.into(), winner.into()],
    ))
    .await?;

    claims::Entity::update_many()
        .col_expr(claims::Column::KnowledgeEntityId, Expr::value(Some(winner)))
        .filter(claims::Column::KnowledgeEntityId.eq(loser))
        .exec(conn)
        .await?;
    // Proximities are a record, so they are settled rather than deleted: the pair naming both ends
    // was the question this merge answers, and any other question about the loser is moot now that
    // it is a tombstone.
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE knowledge_proximities \
            SET verdict = CASE \
                  WHEN (holder_id = $1 OR counterpart_id = $1) \
                       AND (holder_id = $2 OR counterpart_id = $2) THEN 'merged' \
                  ELSE 'moot' \
                END, \
                settled_at = now() \
          WHERE verdict IS NULL AND (holder_id = $1 OR counterpart_id = $1)",
        [loser.into(), winner.into()],
    ))
    .await?;

    // The audit trail: which entity was folded into which, and why. A historical row, so it stays
    // put when either end is later merged again — unlike the collision queue below.
    merges::Entity::insert(merges::ActiveModel {
        loser_id: Set(loser),
        winner_id: Set(winner),
        cause: Set(cause.method().to_string()),
        source: Set(cause.source().to_string()),
        ..Default::default()
    })
    .exec(conn)
    .await?;

    // Tombstone, never delete: a stale pointer still resolves.
    entity::Entity::update_many()
        .col_expr(entity::Column::MergedIntoId, Expr::value(Some(winner)))
        .filter(entity::Column::Id.eq(loser))
        .exec(conn)
        .await?;

    super::project::project(conn, winner).await?;
    Ok(Outcome::Merged)
}

#[cfg(test)]
mod tests {
    use sea_orm::{PaginatorTrait, Set};

    use super::*;
    use crate::model::user_observed_entity as obs;
    use crate::test_support::fixtures;

    async fn entity_with(db: &crate::database::DbHandle, label: &str) -> i32 {
        entity::Entity::insert(entity::ActiveModel {
            name: Set(label.to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set("provisional".to_string()),
            alt_names: Set(vec![label.to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32
    }

    /// The resolution loop can reach one conflict twice, because the loser is still in the run's
    /// entity list after it has been folded. A repeat must be a no-op, not a second merge event.
    #[tokio::test]
    async fn merging_the_same_pair_twice_records_one_event() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        assert_eq!(
            merge(&db.conn, loser, winner, Cause::IdentityConflict)
                .await
                .expect("first"),
            Outcome::Merged
        );
        assert_eq!(
            merge(&db.conn, loser, winner, Cause::IdentityConflict)
                .await
                .expect("second"),
            Outcome::AlreadyMerged
        );
        assert_eq!(
            merges::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one merge, one event"
        );
    }

    /// The loser's row disappears, so anything only it knew has to survive through the members that
    /// moved. Auditing the columns found that labels and descriptions were lost here.
    #[tokio::test]
    async fn the_losers_surfaces_survive_in_the_winner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        let user_id = fixtures::user(&db, "merge_surfaces").await;

        // Each entity needs its own observation: a merge is refused when both sides appear in one
        // capture, so the winner's surface has to come from somewhere real.
        let winner_capture = fixtures::capture(&db, user_id).await;
        let winner_illumination = fixtures::illumination(&db, user_id, winner_capture).await;
        obs::Entity::insert(obs::ActiveModel {
            user_id: Set(user_id),
            capture_id: Set(winner_capture),
            illumination_id: Set(winner_illumination),
            position: Set(0),
            name: Set("Elon Musk".to_string()),
            entity_type: Set("real_person".to_string()),
            knowledge_entity_id: Set(Some(winner)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        obs::Entity::insert(obs::ActiveModel {
            user_id: Set(user_id),
            capture_id: Set(capture_id),
            illumination_id: Set(illumination_id),
            position: Set(0),
            name: Set("Elon".to_string()),
            entity_type: Set("real_person".to_string()),
            description: Set(Some("the CEO of Tesla".to_string())),
            knowledge_entity_id: Set(Some(loser)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        assert_eq!(
            merge(&db.conn, loser, winner, Cause::IdentityConflict)
                .await
                .expect("merge"),
            Outcome::Merged
        );

        let survivor = entity::Entity::find_by_id(winner)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(survivor.name, "Elon Musk", "the winner keeps its surface");
        assert!(
            survivor.alt_names.iter().any(|label| label == "Elon"),
            "the loser's surface is picked up from the member that moved: {:?}",
            survivor.alt_names
        );
        assert_eq!(
            survivor.description.as_deref(),
            Some("the CEO of Tesla"),
            "and so is its description, since the winner had none"
        );
    }

    /// Two entities that both have an observation in one capture cannot be one referent. The veto
    /// decided that at link time; a merge must not contradict it.
    #[tokio::test]
    async fn a_shared_capture_blocks_the_merge() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let book = entity_with(&db, "Corduroy").await;
        let character = entity_with(&db, "Corduroy").await;

        let user_id = fixtures::user(&db, "merge_veto").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        for (position, knowledge_entity_id, name) in
            [(0, book, "Corduroy"), (1, character, "Corduroy")]
        {
            obs::Entity::insert(obs::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(position),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                knowledge_entity_id: Set(Some(knowledge_entity_id)),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }

        let outcome = merge(&db.conn, character, book, Cause::Repair)
            .await
            .expect("merge");

        assert_eq!(outcome, Outcome::RefusedSharedIllumination);
        assert_eq!(
            obs::Entity::find()
                .filter(obs::Column::KnowledgeEntityId.eq(character))
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "nothing moved"
        );
        let survivor = entity::Entity::find_by_id(book)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(survivor.merged_into_id, None, "and nobody was tombstoned");
    }

    #[tokio::test]
    async fn merging_moves_everything_and_tombstones_the_loser() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        let user_id = fixtures::user(&db, "merge").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        obs::Entity::insert(obs::ActiveModel {
            user_id: Set(user_id),
            capture_id: Set(capture_id),
            illumination_id: Set(illumination_id),
            position: Set(0),
            name: Set("Elon".to_string()),
            entity_type: Set("real_person".to_string()),
            knowledge_entity_id: Set(Some(loser)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        fixtures::claim(
            &db,
            loser,
            "wikidata",
            "Q317521",
            "wikidata",
            Some("exact_label"),
            Some(0.8),
        )
        .await;

        let outcome = merge(&db.conn, loser, winner, Cause::IdentityConflict)
            .await
            .expect("merge");
        assert_eq!(outcome, Outcome::Merged);

        assert_eq!(
            obs::Entity::find()
                .filter(obs::Column::KnowledgeEntityId.eq(winner))
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "the observation follows the winner"
        );
        assert_eq!(
            claims::Entity::find()
                .filter(claims::Column::KnowledgeEntityId.eq(winner))
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "the claim moved"
        );
        assert_eq!(
            merges::Entity::find()
                .filter(merges::Column::WinnerId.eq(winner))
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "the merge is recorded"
        );
        let tombstone = entity::Entity::find_by_id(loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tombstone.merged_into_id, Some(winner));
        let survivor = entity::Entity::find_by_id(winner)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(survivor.status, "identified", "the winner was re-projected");
    }

    /// A stale id — the shape a repair command passes — must not "merge" nothing and record an event
    /// for an entity that does not exist.
    #[tokio::test]
    async fn merging_an_entity_that_does_not_exist_is_an_error() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let winner = entity_with(&db, "Elon Musk").await;

        let error = merge(&db.conn, 999_999, winner, Cause::Repair)
            .await
            .expect_err("a missing loser is a caller error");
        assert!(error.to_string().contains("999999"), "{error}");
        assert_eq!(
            merges::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "and nothing was recorded"
        );
    }

    /// The merge row is the only audit trail: it has to say what decided the merge and whose
    /// decision it was, or a repair by hand is indistinguishable from a resolver conflict forever.
    #[tokio::test]
    async fn the_merge_row_records_why() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        for (cause, method, source) in [
            (Cause::IdentityConflict, "identity_conflict", "model"),
            (Cause::Repair, "repair", "user"),
        ] {
            let winner = entity_with(&db, "Elon Musk").await;
            let loser = entity_with(&db, "Elon").await;
            merge(&db.conn, loser, winner, cause).await.unwrap();

            let row = merges::Entity::find()
                .filter(merges::Column::WinnerId.eq(winner))
                .one(&db.conn)
                .await
                .unwrap()
                .expect("a merge row");
            assert_eq!(row.cause, method);
            assert_eq!(row.loser_id, loser);
            assert_eq!(row.source, source);
        }
    }

    /// Merging into a tombstone would strand the loser's observations, so it is refused.
    #[tokio::test]
    async fn merging_into_a_tombstone_is_refused() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let dead = entity_with(&db, "Elon").await;
        let alive = entity_with(&db, "Elon Musk").await;
        merge(&db.conn, dead, alive, Cause::Repair)
            .await
            .expect("the first merge is fine");

        let other = entity_with(&db, "E. Musk").await;
        let error = merge(&db.conn, other, dead, Cause::Repair)
            .await
            .expect_err("the winner is a tombstone");
        assert!(error.to_string().contains("tombstone"), "{error}");

        let loser = entity::Entity::find_by_id(other)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loser.merged_into_id, None, "the loser was left alone");
    }
}
