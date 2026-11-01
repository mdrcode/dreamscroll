//! Merging one core into another.
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
    QueryFilter, Set, Statement,
};

use crate::knowledge::resolve;
use crate::model::{
    knowledge_entity as entity, knowledge_evidence as evidence,
    user_observed_entity as observation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Merged,
    /// Nothing to do: the same pair has already been folded. The resolution loop can reach one
    /// conflict twice — the loser stays in the run's core list after it has been merged — so a
    /// repeat must not move anything again or record a second merge event.
    AlreadyMerged,
    /// The two cores have observations in the same capture. The linker's same-capture veto already
    /// decided that those mentions are different referents, so folding them would contradict a fact
    /// the graph already holds.
    RefusedSharedIllumination,
}

/// What a merge would do, without doing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preview {
    pub observations: u64,
    pub evidence: u64,
    /// Observations the two cores have in the same capture. Nonzero means the merge is refused: the
    /// linker already decided those mentions are different referents.
    pub shared_illuminations: i64,
}

pub async fn preview(
    conn: &DatabaseConnection,
    loser: i32,
    winner: i32,
) -> anyhow::Result<Preview> {
    let observations = observation::Entity::find()
        .filter(observation::Column::KnowledgeEntityId.eq(loser))
        .count(conn)
        .await?;
    let evidence_count = evidence::Entity::find()
        .filter(evidence::Column::KnowledgeEntityId.eq(loser))
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
        evidence: evidence_count,
        shared_illuminations,
    })
}

/// Fold `loser` into `winner`.
///
/// The winner should be the better-substantiated core — in practice the one that already holds the
/// identity which caused the conflict. Both must be live; merging a tombstone is a no-op.
///
/// **The same-illumination veto applies here too.** Two cores that both appear in one *illumination*
/// cannot be one referent: one extractor writing the same string twice in one screenshot means two
/// things, and the linker refused to fold those observations for exactly that reason. A merge that
/// ignored it produced four observations on one core, two of them from the same capture — the state
/// the veto exists to prevent. In that case the conflict is real and needs adjudication, so it is
/// reported rather than resolved.
///
/// The unit is the illumination, not the capture: re-reading an image is a separate reading, and
/// its names may legitimately join the same entity. See `plan/knowledge-illumination-history.md`.
pub async fn merge_core(
    conn: &DatabaseConnection,
    loser: i32,
    winner: i32,
) -> anyhow::Result<Outcome> {
    if loser == winner {
        return Ok(Outcome::AlreadyMerged);
    }
    if entity::Entity::find_by_id(loser)
        .one(conn)
        .await?
        .is_some_and(|row| row.merged_into_id.is_some())
    {
        return Ok(Outcome::AlreadyMerged);
    }

    if preview(conn, loser, winner).await?.shared_illuminations > 0 {
        return Ok(Outcome::RefusedSharedIllumination);
    }

    // Observations follow the winner. This is the only column a merge touches on them.
    observation::Entity::update_many()
        .col_expr(observation::Column::KnowledgeEntityId, Expr::value(Some(winner)))
        .filter(observation::Column::KnowledgeEntityId.eq(loser))
        .exec(conn)
        .await?;

    // Evidence that duplicates the winner's would violate the identity index on the way over, so it
    // is dropped first. It is the same fact stated twice, and the winner's copy is the one kept.
    conn.execute_unprepared(&format!(
        "DELETE FROM knowledge_evidence l USING knowledge_evidence w \
         WHERE l.knowledge_entity_id = {loser} AND w.knowledge_entity_id = {winner} \
           AND l.kind = 'claim' AND w.kind = 'claim' \
           AND w.retracted_at IS NULL \
           AND l.value->>'scheme' IS NOT DISTINCT FROM w.value->>'scheme' \
           AND l.value->>'id' IS NOT DISTINCT FROM w.value->>'id'"
    ))
    .await?;

    evidence::Entity::update_many()
        .col_expr(evidence::Column::KnowledgeEntityId, Expr::value(Some(winner)))
        .filter(evidence::Column::KnowledgeEntityId.eq(loser))
        .exec(conn)
        .await?;

    // The audit trail: which core was folded into which, and why.
    evidence::Entity::insert(evidence::ActiveModel {
        knowledge_entity_id: Set(winner),
        kind: Set("merge".to_string()),
        value: Set(serde_json::json!({ "child": loser, "method": "identity_conflict" })),
        source: Set("model".to_string()),
        ..Default::default()
    })
    .exec(conn)
    .await?;

    // Tombstone, never delete: a stale pointer still resolves.
    entity::Entity::update_many()
        .col_expr(
            entity::Column::MergedIntoId,
            Expr::value(Some(winner)),
        )
        .filter(entity::Column::Id.eq(loser))
        .exec(conn)
        .await?;

    resolve::project(conn, winner).await?;
    Ok(Outcome::Merged)
}

#[cfg(test)]
mod tests {
    use sea_orm::{PaginatorTrait, Set};

    use super::*;
    use crate::model::user_observed_entity as obs;
    use crate::test_support::fixtures;

    async fn core_with(db: &crate::database::DbHandle, label: &str) -> i32 {
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
    /// core list after it has been folded. A repeat must be a no-op, not a second merge event.
    #[tokio::test]
    async fn merging_the_same_pair_twice_records_one_event() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let winner = core_with(&db, "Elon Musk").await;
        let loser = core_with(&db, "Elon").await;

        assert_eq!(
            merge_core(&db.conn, loser, winner).await.expect("first"),
            Outcome::Merged
        );
        assert_eq!(
            merge_core(&db.conn, loser, winner).await.expect("second"),
            Outcome::AlreadyMerged
        );
        assert_eq!(
            evidence::Entity::find()
                .filter(evidence::Column::Kind.eq("merge"))
                .count(&db.conn)
                .await
                .unwrap(),
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
        let winner = core_with(&db, "Elon Musk").await;
        let loser = core_with(&db, "Elon").await;

        let user_id = fixtures::user(&db, "merge_surfaces").await;

        // Each core needs its own observation: a merge is refused when both sides appear in one
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
            merge_core(&db.conn, loser, winner).await.expect("merge"),
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

    /// Two cores that both have an observation in one capture cannot be one referent. The veto
    /// decided that at link time; a merge must not contradict it.
    #[tokio::test]
    async fn a_shared_capture_blocks_the_merge() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let book = core_with(&db, "Corduroy").await;
        let character = core_with(&db, "Corduroy").await;

        let user_id = fixtures::user(&db, "merge_veto").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let illumination_id = fixtures::illumination(&db, user_id, capture_id).await;
        for (position, knowledge_entity_id, name) in [(0, book, "Corduroy"), (1, character, "Corduroy")] {
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

        let outcome = merge_core(&db.conn, character, book).await.expect("merge");

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
        let winner = core_with(&db, "Elon Musk").await;
        let loser = core_with(&db, "Elon").await;

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

        evidence::Entity::insert(evidence::ActiveModel {
            knowledge_entity_id: Set(loser),
            kind: Set("claim".to_string()),
            value: Set(serde_json::json!({"scheme": "wikidata", "id": "Q317521"})),
            source: Set("wikidata".to_string()),
            method: Set(Some("exact_label".to_string())),
            confidence: Set(Some(0.8)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        let outcome = merge_core(&db.conn, loser, winner).await.expect("merge");
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
            evidence::Entity::find()
                .filter(evidence::Column::KnowledgeEntityId.eq(winner))
                .filter(evidence::Column::Kind.eq("claim"))
                .count(&db.conn)
                .await
                .unwrap(),
            1,
            "the evidence moved"
        );
        assert_eq!(
            evidence::Entity::find()
                .filter(evidence::Column::KnowledgeEntityId.eq(winner))
                .filter(evidence::Column::Kind.eq("merge"))
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
}
