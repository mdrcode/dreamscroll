//! Replay: apply the recorded decisions to a rebuilt graph.

use std::collections::BTreeMap;

use crate::knowledge::evidence::IdentityKey;
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
    /// Pairs a rule refused: the shared-illumination rule, or a standing `distinct` verdict.
    pub refused: usize,
    /// Identity annotations deleted because an entity was vetoed out of an identity.
    pub withdrawn: usize,
    /// Vetoes already in force: the pairing has no annotation to delete — never written, or deleted
    /// by an earlier replay. A settled decision is not a failure.
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
            Outcome::RefusedVerdict => report.refused += 1,
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

/// Delete the vetoed identity annotation for its entity, then reproject it.
///
/// The veto names a **pairing**, so the deletion does too: every provider's row for
/// `(entity, identity)` goes. `None` when the subject origin no longer names a live entity; `Some(0)`
/// when the decision already holds and there is nothing to delete.
///
/// The deletion and the projection are one act: `status` and `confidence` are functions of the
/// identity annotations, so withdrawing one without reprojecting leaves the entity `identified` on
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
    Ok(Some(
        withdraw_entity(graph, entity_id, scheme, external).await?,
    ))
}

/// Delete the identity annotation for `(entity, identity)` and reproject the entity, if either
/// exists.
///
/// The entity is followed through tombstones first: a veto may name a row that a merge has already
/// folded, and the annotation now hangs off the survivor.
pub(super) async fn withdraw_entity(
    graph: &KnowledgeGraph,
    entity_id: i32,
    scheme: &str,
    external: &str,
) -> anyhow::Result<usize> {
    let Some(entity_id) = graph.verdicts.canonical(entity_id).await? else {
        return Ok(0);
    };
    let key = IdentityKey {
        scheme: scheme.to_string(),
        external_id: external.to_string(),
    };
    let withdrawn = graph.evidence.delete_identity(entity_id, &key).await?;
    if withdrawn > 0 {
        graph.update_projection(entity_id).await?;
    }
    Ok(withdrawn)
}
