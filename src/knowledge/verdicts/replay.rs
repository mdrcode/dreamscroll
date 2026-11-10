//! Replay: apply the recorded decisions to a rebuilt graph.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::knowledge::graph::merge::Outcome;
use crate::knowledge::graph::KnowledgeGraph;
use crate::model;
use crate::model::knowledge_verdicts::Kind;

use super::Cause;
use super::store::{VerdictStore, pair_key};

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

impl VerdictStore {
    /// Apply the recorded verdicts to the current graph, newest verdict per pair first.
    pub async fn replay(&self, graph: &KnowledgeGraph) -> anyhow::Result<ReplayReport> {
        let conn = &self.conn;
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
            latest.insert(pair_key(winner, loser), verdict);
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
                self.entity_with_origin(winner_origin).await?,
                self.entity_with_origin(loser_origin).await?,
            ) else {
                report.gone += 1;
                continue;
            };
            // Follow the tombstones: the model may have folded the person's winner into something
            // else, and the decision still means "these two are one" about wherever they are now.
            let (Some(winner), Some(loser)) = (
                self.canonical(winner.id).await?,
                self.canonical(loser.id).await?,
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
            match self.withdraw(graph, &verdict).await? {
                Some(0) => report.settled += 1,
                Some(retracted) => report.withdrawn += retracted,
                None => report.gone += 1,
            }
        }
        Ok(report)
    }
}
