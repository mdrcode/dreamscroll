//! Durable human decisions: which two things are one, kept across rebuilds.
//!
//! A merge is entity → entity in the present tense. A rebuild renumbers entities, so the record is
//! keyed by each side's `origin_observation_id` — observations are never deleted or renumbered, and
//! `link` builds entities in a deterministic order, so the origin is a stable name for a thing.
//!
//! Replay asks the graph what each origin names now and folds the pairs that still map. Anything
//! that does not is reported and dropped: erosion is the price of the key, and a silent wrong merge
//! is not.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::model;

use super::merge::{Cause, Outcome};
use super::Knowledge;

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
}

/// The generation the graph is being built in. One is implicit on a database that never reset.
pub(crate) async fn current_generation<C: ConnectionTrait>(conn: &C) -> anyhow::Result<i32> {
    Ok(model::knowledge_generations::Entity::find()
        .order_by_desc(model::knowledge_generations::Column::Id)
        .one(conn)
        .await?
        .map_or(1, |row| row.id))
}

/// The unordered pair two origins name, as a key.
pub(crate) fn pair_key(a: i32, b: i32) -> (i32, i32) {
    (a.min(b), a.max(b))
}

/// Record a merge. Idempotent per cause, generation and direction, so replaying a repair does not
/// grow the log and an automatic fold is only ever noted once in the generation it happened in.
pub(super) async fn record<C: ConnectionTrait>(
    conn: &C,
    cause: Cause,
    winner: &model::knowledge_entity::Model,
    loser: &model::knowledge_entity::Model,
) -> anyhow::Result<bool> {
    let (Some(winner_origin), Some(loser_origin)) =
        (winner.origin_observation_id, loser.origin_observation_id)
    else {
        // A row without an origin cannot be named after a rebuild. Tests and hand-seeded rows can
        // lack one; a merge they take part in simply does not enter the log.
        return Ok(false);
    };
    let generation = current_generation(conn).await?;
    let existing = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(cause.as_str()))
        .filter(model::knowledge_verdicts::Column::Generation.eq(generation))
        .filter(model::knowledge_verdicts::Column::WinnerOrigin.eq(winner_origin))
        .filter(model::knowledge_verdicts::Column::LoserOrigin.eq(loser_origin))
        .one(conn)
        .await?;
    if existing.is_some() {
        return Ok(false);
    }
    model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
        generation: Set(generation),
        cause: Set(cause.as_str().to_string()),
        winner_origin: Set(winner_origin),
        loser_origin: Set(loser_origin),
        winner_label: Set(Some(winner.name.clone())),
        loser_label: Set(Some(loser.name.clone())),
        ..Default::default()
    })
    .exec(conn)
    .await?;
    Ok(true)
}

/// Apply the recorded verdicts to the current graph, newest verdict per pair first.
pub async fn replay(knowledge: &Knowledge) -> anyhow::Result<ReplayReport> {
    // Only a person's decisions are replayed. The model's folds and earlier replays are history.
    let verdicts = model::knowledge_verdicts::Entity::find()
        .filter(model::knowledge_verdicts::Column::Cause.eq(Cause::Repair.as_str()))
        .order_by_asc(model::knowledge_verdicts::Column::Id)
        .all(knowledge.conn())
        .await?;

    // Latest by id wins for a pair, and chains still apply oldest first.
    let mut latest: BTreeMap<(i32, i32), model::knowledge_verdicts::Model> = BTreeMap::new();
    for verdict in verdicts {
        latest.insert(pair_key(verdict.winner_origin, verdict.loser_origin), verdict);
    }
    let mut ordered: Vec<model::knowledge_verdicts::Model> = latest.into_values().collect();
    ordered.sort_by_key(|verdict| verdict.id);

    let mut report = ReplayReport::default();
    for verdict in ordered {
        // Tombstones count: a verdict applied in this generation leaves the loser's row behind,
        // pointing at the winner. "Already" and "gone" must not be confused.
        let (Some(winner), Some(loser)) = (
            entity_with_origin(knowledge, verdict.winner_origin).await?,
            entity_with_origin(knowledge, verdict.loser_origin).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        // Follow the tombstones: the model may have folded the person's winner into something else,
        // and the decision still means "these two are one" about wherever they are now.
        let (Some(winner), Some(loser)) = (
            canonical(knowledge, winner.id).await?,
            canonical(knowledge, loser.id).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        if winner == loser {
            report.already += 1;
            continue;
        }
        match knowledge.merge(loser, winner, Cause::Replayed).await? {
            Outcome::Merged => report.applied += 1,
            Outcome::AlreadyMerged => report.already += 1,
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }
    Ok(report)
}

/// The entity this origin names now, live or tombstoned.
async fn entity_with_origin(
    knowledge: &Knowledge,
    origin: i32,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    Ok(model::knowledge_entity::Entity::find()
        .filter(model::knowledge_entity::Column::OriginObservationId.eq(origin))
        .one(knowledge.conn())
        .await?)
}

/// Follow tombstones to the live entity a row ends at. Bounded: a cycle is not expressible in the
/// schema, but the reader must not spin if one somehow appears.
async fn canonical(knowledge: &Knowledge, id: i32) -> anyhow::Result<Option<i32>> {
    let mut current = id;
    for _ in 0..8 {
        let Some(row) = model::knowledge_entity::Entity::find_by_id(current)
            .one(knowledge.conn())
            .await?
        else {
            return Ok(None);
        };
        match row.merged_into_id {
            Some(next) => current = next,
            None => return Ok(Some(current)),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    use crate::test_support::{db_or_skip, fixtures};

    use super::*;

    /// The automatic pass logs its fold with its own cause, and replay leaves it alone: the model's
    /// opinion is re-derived every build, never restored.
    #[tokio::test]
    async fn an_automatic_merge_is_logged_but_not_replayed() {
        db_or_skip!(db);
        let store = Knowledge::new(&db);
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
        assert_eq!(logged[0].cause, "proximity_auto");

        let report = replay(&store).await.expect("replay");
        assert_eq!(report, ReplayReport::default(), "nothing to replay");
    }

    /// Replay folds what a previous build had merged, by origin.
    #[tokio::test]
    async fn replay_folds_the_pair_again() {
        db_or_skip!(db);
        let store = Knowledge::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = origin(&db, winner).await;
        let loser_origin = origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild: the graph is emptied, the observations and the verdict survive it.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        set_origin(&db, reborn_winner, winner_origin).await;
        set_origin(&db, reborn_loser, loser_origin).await;

        let report = replay(&store).await.expect("replay");

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
        let store = Knowledge::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let winner_origin = origin(&db, winner).await;
        let loser_origin = origin(&db, loser).await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        // A rebuild gives the pair back, then the model folds the person's winner into a third
        // entity: the winner is now a tombstone, and the loser is not.
        crate::knowledge::snapshot::reset(&db).await.unwrap();
        let reborn_winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let reborn_loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        set_origin(&db, reborn_winner, winner_origin).await;
        set_origin(&db, reborn_loser, loser_origin).await;
        let heir = fixtures::knowledge_entity_with_member(&db, "Train Dreams (film)").await;
        store
            .merge(reborn_winner, heir, Cause::ProximityAuto)
            .await
            .unwrap();

        let report = replay(&store).await.expect("replay");

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
        let store = Knowledge::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        store.merge(loser, winner, Cause::Repair).await.unwrap();

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.already, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
        assert_eq!(report.gone, 0, "{report:?}");
    }

    /// A verdict whose sides are not both live is reported, not guessed.
    #[tokio::test]
    async fn replay_reports_sides_that_are_gone() {
        db_or_skip!(db);
        let store = Knowledge::new(&db);
        let only = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let origin = origin(&db, only).await;
        model::knowledge_verdicts::Entity::insert(model::knowledge_verdicts::ActiveModel {
            generation: Set(1),
            winner_origin: Set(origin),
            loser_origin: Set(origin + 1000),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        let report = replay(&store).await.expect("replay");

        assert_eq!(report.gone, 1, "{report:?}");
        assert_eq!(report.applied, 0, "{report:?}");
    }

    async fn origin(db: &crate::database::DbHandle, id: i32) -> i32 {
        model::knowledge_entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .origin_observation_id
            .expect("fixtures set the origin")
    }

    async fn set_origin(db: &crate::database::DbHandle, id: i32, origin: i32) {
        model::knowledge_entity::Entity::update_many()
            .col_expr(
                model::knowledge_entity::Column::OriginObservationId,
                sea_orm::sea_query::Expr::value(Some(origin)),
            )
            .filter(model::knowledge_entity::Column::Id.eq(id))
            .exec(&db.conn)
            .await
            .unwrap();
    }

}
