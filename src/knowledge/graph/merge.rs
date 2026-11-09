//! Merging one entity into another.
//!
//! A merge is the operation the whole model is built to make cheap: the loser's observations are
//! re-pointed, its evidence moves, and the loser becomes a tombstone. Nothing is edited and nothing is
//! deleted, so the decision is one pointer away from being undone.
//!
//! The primitive behind both the identity-conflict path and `dreamscroll_knowledge merge`.

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, Statement, TransactionTrait,
};

use crate::model::{
    knowledge_claims as claims, knowledge_entity as entity,
    user_observed_entity as observation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Merged,
    /// Nothing to do: the same pair has already been folded. The resolution loop can reach one
    /// conflict twice — the loser stays in the run's entity list after it has been merged — so a
    /// repeat must not move anything again or record a second merge event.
    AlreadyMerged,
    /// Both entities have observations in one illumination. One extraction emitting both is its own
    /// statement that they are two things; folding them would contradict it, so this is reported.
    RefusedSharedIllumination,
}

/// What a merge would do, without doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preview {
    pub observations: u64,
    pub evidence: u64,
    /// Observations the two entities have in one illumination. Nonzero means the merge is refused.
    pub shared_illuminations: i64,
}

/// Why a merge happened — and, because the same word covers both, whose decision it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The automatic pass folded a proximity candidate. The model's opinion, re-derived every build.
    ProximityAuto,
    /// A person decided these are one thing. Kept, and replayed in every later generation.
    Repair,
    /// An older repair, applied to this generation by the replay.
    Replayed,
}

impl Cause {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProximityAuto => "proximity_auto",
            Self::Repair => "repair",
            Self::Replayed => "replayed",
        }
    }
}

/// How many illuminations both entities appear in.
async fn shared_illuminations<C: ConnectionTrait>(
    conn: &C,
    loser: i32,
    winner: i32,
) -> anyhow::Result<i64> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) FROM user_observed_entities l \
             JOIN user_observed_entities w ON w.illumination_id = l.illumination_id \
             WHERE l.knowledge_entity_id = $1 AND w.knowledge_entity_id = $2",
            [loser.into(), winner.into()],
        ))
        .await?;
    Ok(row
        .map(|row| row.try_get_by_index::<i64>(0))
        .transpose()?
        .unwrap_or(0))
}

/// Fold `loser` into `winner`. See [`merge`] for what that means and why it can refuse.
///
/// The blocker is told afterwards, not inside the transaction: projecting the winner happens within
/// it, but a rollback would leave the index holding a row that never landed. The second projection is
/// idempotent and merges are rare, so it doubles as the notification.
pub(super) async fn merge_into(
    conn: &sea_orm::DatabaseConnection,
    graph: &super::KnowledgeGraph,
    loser: i32,
    winner: i32,
    cause: Cause,
) -> anyhow::Result<Outcome> {
    let outcome = merge(conn, loser, winner, cause).await?;
    if outcome == Outcome::Merged {
        graph.update_projection(winner).await?;
        graph.forget(loser);
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
    let shared_illuminations = shared_illuminations(conn, loser, winner).await?;
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
/// Two entities that both appear in one *illumination* cannot be folded: one extractor emitting both
/// is its own statement that they are two things, and a merge must not contradict it. The unit is the
/// illumination, not the capture — re-illuminating an image makes a separate one, whose names may
/// legitimately join. See `plan/knowledge-illumination-history.md`.
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

    if shared_illuminations(conn, loser, winner).await? > 0 {
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
    // The log: which thing was folded into which, and what decided it. One row per merge, keyed by
    // what survives a rebuild.
    if let (Some(winner_row), Some(loser_row)) = (
        rows.iter().find(|row| row.id == winner),
        rows.iter().find(|row| row.id == loser),
    ) {
        super::verdicts::record(conn, cause, winner_row, loser_row).await?;
    }

    // Tombstone, never delete: a stale pointer still resolves.
    entity::Entity::update_many()
        .col_expr(entity::Column::MergedIntoId, Expr::value(Some(winner)))
        .filter(entity::Column::Id.eq(loser))
        .exec(conn)
        .await?;
    // A tombstone has no surfaces to block on, so its retrieval material goes with it.
    crate::knowledge::blocker::index::forget(conn, loser).await?;

    super::project::project(conn, winner).await?;
    Ok(Outcome::Merged)
}

#[cfg(test)]
mod tests {
    use sea_orm::{PaginatorTrait, Set};

    use super::*;
    use crate::model::knowledge_verdicts as verdicts;
    use crate::model::user_observed_entity as obs;
    use crate::test_support::fixtures;
    use crate::test_support::db_or_skip;

    async fn entity_with(db: &crate::database::DbHandle, label: &str) -> i32 {
        let id = entity::Entity::insert(entity::ActiveModel {
            name: Set(label.to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(crate::model::knowledge_entity::Status::Provisional),
            alt_names: Set(vec![label.to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;
        // The log keys a merge by origin, so a test entity needs one.
        fixtures::set_origin(db, id, 10_000_000 + id).await;
        id
    }

    /// The resolution loop can reach one conflict twice, because the loser is still in the run's
    /// entity list after it has been folded. A repeat must be a no-op, not a second merge event.
    #[tokio::test]
    async fn merging_the_same_pair_twice_records_one_event() {
        db_or_skip!(db);
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        assert_eq!(
            merge(&db.conn, loser, winner, Cause::ProximityAuto)
                .await
                .expect("first"),
            Outcome::Merged
        );
        assert_eq!(
            merge(&db.conn, loser, winner, Cause::ProximityAuto)
                .await
                .expect("second"),
            Outcome::AlreadyMerged
        );
        assert_eq!(
            verdicts::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one merge, one event"
        );
    }

    /// The loser's row disappears, so anything only it knew has to survive through the members that
    /// moved. Auditing the columns found that labels and descriptions were lost here.
    #[tokio::test]
    async fn the_losers_surfaces_survive_in_the_winner() {
        db_or_skip!(db);
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        // Each entity needs its own capture: a merge is refused when both sides appear in one, so
        // the winner's surface has to come from somewhere real.
        let winner_capture = fixtures::Capture::new(&db, "merge_surfaces_winner").await;
        fixtures::attached(
            &db,
            &winner_capture,
            0,
            "Elon Musk",
            None,
            "real_person",
            winner,
        )
        .await;
        let loser_capture = fixtures::Capture::new(&db, "merge_surfaces_loser").await;
        fixtures::attached(
            &db,
            &loser_capture,
            0,
            "Elon",
            Some("the CEO of Tesla"),
            "real_person",
            loser,
        )
        .await;

        assert_eq!(
            merge(&db.conn, loser, winner, Cause::ProximityAuto)
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
        db_or_skip!(db);
        let book = entity_with(&db, "Corduroy").await;
        let character = entity_with(&db, "Corduroy").await;

        let capture = fixtures::Capture::new(&db, "merge_veto").await;
        for (position, knowledge_entity_id, name) in
            [(0, book, "Corduroy"), (1, character, "Corduroy")]
        {
            fixtures::attached(&db, &capture, position, name, None, "book", knowledge_entity_id).await;
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
        db_or_skip!(db);
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        let capture = fixtures::Capture::new(&db, "merge").await;
        fixtures::attached(&db, &capture, 0, "Elon", None, "real_person", loser).await;

        fixtures::claim(
            &db,
            loser,
            "wikidata",
            "Q317521",
            "wikidata",
            Some("exact_label"),
            Some(0.8),
            crate::model::knowledge_claims::ClaimBasis::Text,
        )
        .await;

        let outcome = merge(&db.conn, loser, winner, Cause::ProximityAuto)
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
            verdicts::Entity::find()
                .filter(verdicts::Column::WinnerOrigin.eq(fixtures::origin(&db, winner).await))
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
        assert_eq!(survivor.status, crate::model::knowledge_entity::Status::Identified, "the winner was re-projected");
    }

    /// A stale id — the shape a repair command passes — must not "merge" nothing and record an event
    /// for an entity that does not exist.
    #[tokio::test]
    async fn merging_an_entity_that_does_not_exist_is_an_error() {
        db_or_skip!(db);
        let winner = entity_with(&db, "Elon Musk").await;

        let error = merge(&db.conn, 999_999, winner, Cause::Repair)
            .await
            .expect_err("a missing loser is a caller error");
        assert!(error.to_string().contains("999999"), "{error}");
        assert_eq!(
            verdicts::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "and nothing was recorded"
        );
    }

    /// The log is the only audit trail: it has to say what decided the merge and which way, or a
    /// repair by hand is indistinguishable from an automatic fold forever.
    #[tokio::test]
    async fn the_log_records_why_and_which_way() {
        db_or_skip!(db);
        let winner = entity_with(&db, "Elon Musk").await;
        let loser = entity_with(&db, "Elon").await;

        merge(&db.conn, loser, winner, Cause::Repair)
            .await
            .unwrap();

        let row = verdicts::Entity::find()
            .filter(verdicts::Column::WinnerOrigin.eq(fixtures::origin(&db, winner).await))
            .one(&db.conn)
            .await
            .unwrap()
            .expect("a log row");
        assert_eq!(row.cause, "repair");
        assert_eq!(row.loser_origin, Some(fixtures::origin(&db, loser).await));
    }

    /// Three causes, three authorities; the column is read by the replay, so the spelling is API.
    #[test]
    fn every_cause_names_itself() {
        for (cause, name) in [
            (Cause::Repair, "repair"),
            (Cause::ProximityAuto, "proximity_auto"),
            (Cause::Replayed, "replayed"),
        ] {
            assert_eq!(cause.as_str(), name);
        }
    }

    /// Merging into a tombstone would strand the loser's observations, so it is refused.
    #[tokio::test]
    async fn merging_into_a_tombstone_is_refused() {
        db_or_skip!(db);
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
