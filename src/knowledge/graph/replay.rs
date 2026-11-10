//! Replay: apply the recorded decisions to a rebuilt graph.

use std::collections::BTreeMap;

use crate::knowledge::verdicts::{Cause, pair_key};
use crate::model;

use super::KnowledgeGraph;
use super::merge::Outcome;

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
pub(crate) async fn replay(graph: &KnowledgeGraph) -> anyhow::Result<ReplayReport> {
    let merges = graph.verdicts.merges().await?;

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
            graph.verdicts.entity_with_origin(winner_origin).await?,
            graph.verdicts.entity_with_origin(loser_origin).await?,
        ) else {
            report.gone += 1;
            continue;
        };
        // Follow the tombstones: the model may have folded the person's winner into something
        // else, and the decision still means "these two are one" about wherever they are now.
        let (Some(winner), Some(loser)) = (
            graph.verdicts.canonical(winner.id).await?,
            graph.verdicts.canonical(loser.id).await?,
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
    for verdict in graph.verdicts.vetoes().await? {
        match withdraw(graph, &verdict).await? {
            Some(0) => report.settled += 1,
            Some(retracted) => report.withdrawn += retracted,
            None => report.gone += 1,
        }
    }
    Ok(report)
}

/// Retract the live claim holding a vetoed identity for its entity, then reproject it.
///
/// The veto names a **pairing**, so the retraction does too: there is at most one live claim for
/// `(entity, identity)`, whatever route produced it. `None` when the subject origin no longer names
/// a live entity; `Some(0)` when the decision already holds and there is nothing to retract.
///
/// The retraction and the projection are one act: `status` and `confidence` are functions of the
/// live claims, so withdrawing a claim without reprojecting leaves the entity `identified` on
/// nothing — and that is the row the API serves and `proximity auto_merge` ranks.
async fn withdraw(
    graph: &KnowledgeGraph,
    verdict: &model::knowledge_verdicts::Model,
) -> anyhow::Result<Option<usize>> {
    let (Some(subject), Some(scheme), Some(external)) = (
        verdict.subject_origin,
        verdict.identity_scheme.as_deref(),
        verdict.identity_external_id.as_deref(),
    ) else {
        return Ok(Some(0));
    };
    let Some(row) = graph.verdicts.entity_with_origin(subject).await? else {
        return Ok(None);
    };
    let Some(entity_id) = graph.verdicts.canonical(row.id).await? else {
        return Ok(None);
    };
    let Some(identity_id) = graph.evidence.identity_id(scheme, external).await? else {
        return Ok(Some(0));
    };
    let retracted = graph.evidence.retract(entity_id, identity_id).await?;
    if retracted > 0 {
        graph.update_projection(entity_id).await?;
    }
    Ok(Some(retracted))
}
