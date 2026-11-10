//! Replay: apply a person's recorded decisions to a rebuilt graph.
//!
//! A merge is entity → entity in the present tense. A rebuild renumbers entities, so the record is
//! keyed by each side's `origin_observation_id` — observations are never deleted or renumbered, and
//! `link` builds entities in a deterministic order, so the origin is a stable name for a thing.
//!
//! Replay asks the graph what each origin names now and folds the pairs that still map. Anything
//! that does not is reported and dropped: erosion is the price of the key, and a silent wrong merge
//! is not.
//!
//! It is a command over the whole graph, so it takes what it acts on: the connection it reads the
//! log through, and the store it folds into.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::knowledge::graph::merge::Outcome;
use crate::knowledge::graph::{verdicts, KnowledgeGraph};
use crate::model;
use crate::model::knowledge_verdicts::{Cause, Kind};

/// What one replay did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    /// Pairs folded onto the current graph.
    pub applied: usize,
    /// Pairs the graph already had as one entity.
    pub already: usize,
    /// Verdicts whose sides no longer name a live entity each.
    pub gone: usize,
    /// Pairs the shared-illumination rule refused.
    pub refused: usize,
    /// Live claims withdrawn because an entity was vetoed out of an identity.
    pub withdrawn: usize,
    /// Vetoes already in force: the pairing has no live claim to retract — never written, or
    /// retracted by an earlier replay. A settled decision is not a failure.
    pub settled: usize,
}

/// Apply the recorded verdicts to the current graph, newest verdict per pair first.
pub async fn replay(
    conn: &DatabaseConnection,
    graph: &KnowledgeGraph,
) -> anyhow::Result<ReplayReport> {
    // Only a person's decisions are replayed. The model's folds and earlier replays are history.
    let merges = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Merge.as_str()))
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(conn)
        .await?;

    // Latest by id wins for a pair, and chains still apply oldest first.
    let mut latest: BTreeMap<(i32, i32), model::knowledge_verdicts::Model> = BTreeMap::new();
    for verdict in merges {
        let (Some(winner), Some(loser)) = (verdict.winner_origin, verdict.loser_origin) else {
            continue;
        };
        latest.insert(verdicts::pair_key(winner, loser), verdict);
    }
    let mut ordered: Vec<model::knowledge_verdicts::Model> = latest.into_values().collect();
    ordered.sort_by_key(|verdict| verdict.id);

    let mut report = ReplayReport::default();
    for verdict in ordered {
        // Tombstones count: a verdict applied in this generation leaves the loser's row behind,
        // pointing at the winner. "Already" and "gone" must not be confused.
        let (Some(winner_origin), Some(loser_origin)) =
            (verdict.winner_origin, verdict.loser_origin)
        else {
            continue;
        };
        let (Some(winner), Some(loser)) = (
            verdicts::entity_with_origin(conn, winner_origin).await?,
            verdicts::entity_with_origin(conn, loser_origin).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        // Follow the tombstones: the model may have folded the person's winner into something else,
        // and the decision still means "these two are one" about wherever they are now.
        let (Some(winner), Some(loser)) = (
            verdicts::canonical(conn, winner.id).await?,
            verdicts::canonical(conn, loser.id).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        if winner == loser {
            report.already += 1;
            continue;
        }
        match graph.merge(loser, winner, Cause::Replayed).await? {
            Outcome::Merged => report.applied += 1,
            Outcome::AlreadyMerged => report.already += 1,
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }

    // Vetoes last. A merge decides *which* entity a verdict is about, so a veto is applied to
    // wherever its subject ended up, not to the entity the editor happened to be looking at.
    let vetoes = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .filter(model::knowledge_verdicts::Column::Kind.eq(Kind::Veto.as_str()))
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(conn)
        .await?;
    for verdict in vetoes {
        match verdicts::withdraw(graph, &verdict).await? {
            Some(0) => report.settled += 1,
            Some(retracted) => report.withdrawn += retracted,
            None => report.gone += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use sea_orm::{EntityTrait, Set};

    use crate::model;
    use crate::test_support::{db_or_skip, fixtures};

    use super::*;

    /// The automatic pass logs its fold with its own cause, and replay leaves it alone: the model's
    /// opinion is re-derived every build, never restored.
    #[tokio::test]
    async fn an_automatic_merge_is_logged_but_not_replayed() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;

        store
            .merge(loser, winner, Cause::ProximityAuto)
            .await
            .unwrap();

        let logged = model::knowledge_verdicts::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].cause, crate::model::knowledge_verdicts::Cause::ProximityAuto);

        let report = replay(&db.conn, &store).await.expect("replay");
        assert_eq!(report, ReplayReport::default(), "nothing to replay");
    }

    /// Replay folds what a previous build had merged, by origin.
    #[tokio::test]
    async fn replay_folds_the_pair_again() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = fixtures::origin(&db, winner).await;
        let loser_origin = fixtures::origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild: the graph is emptied, the observations and the verdict survive it.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::set_origin(&db, reborn_winner, winner_origin).await;
        fixtures::set_origin(&db, reborn_loser, loser_origin).await;

        let report = replay(&db.conn, &store).await.expect("replay");

        assert_eq!(report.applied, 1, "{report:?}");
        let tombstone = model::knowledge_entity::Entity::find_by_id(reborn_loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(tombstone.merged_into_id.is_some(), "the recorded side folded");
    }

    /// The model may fold a person's winner into something else. The decision still means "these two
    /// are one", so the replay follows the tombstone and folds into wherever the winner ended up.
    #[tokio::test]
    async fn replay_follows_a_winner_the_model_folded_away() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = fixtures::origin(&db, winner).await;
        let loser_origin = fixtures::origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild gives the pair back, then the model folds the person's winner into a third
        // entity: the winner is now a tombstone, and the loser is not.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        fixtures::set_origin(&db, reborn_winner, winner_origin).await;
        fixtures::set_origin(&db, reborn_loser, loser_origin).await;
        let heir = fixtures::knowledge_entity_with_member(&db, "Train Dreams (film)").await;
        store
            .merge(reborn_winner, heir, Cause::ProximityAuto)
            .await
            .unwrap();

        let report = replay(&db.conn, &store).await.expect("replay");

        assert_eq!(report.applied, 1, "{report:?}");
        assert_eq!(report.gone, 0, "{report:?}");
        let tombstone = model::knowledge_entity::Entity::find_by_id(reborn_loser)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tombstone.merged_into_id,
            Some(heir),
            "the decision retargets through the winner's tombstone"
        );
    }

    /// Applied once, reported as already applied on every later run — a tombstone is not erosion.
    #[tokio::test]
    async fn replay_is_idempotent() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        let report = replay(&db.conn, &store).await.expect("replay");

        assert_eq!(report.already, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
        assert_eq!(report.gone, 0, "{report:?}");
    }

    /// A verdict whose sides are not both live is reported, not guessed.
    #[tokio::test]
    async fn replay_reports_sides_that_are_gone() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let only = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let origin = fixtures::origin(&db, only).await;
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            winner_origin: Set(Some(origin)),
            loser_origin: Set(Some(origin + 1000)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        let report = replay(&db.conn, &store).await.expect("replay");

        assert_eq!(report.gone, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
    }
}
