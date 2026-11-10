//! Answering the batch: fold what scores as one thing, leave the rest as questions.

use std::cmp::Ordering;

use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use crate::knowledge::graph::merge::{Cause, Outcome};
use crate::knowledge::graph::KnowledgeGraph;
use crate::knowledge::scoring::{Scoring, TotalCmp};

/// What one merge did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Pairs this pass merged on its own — logged `proximity_auto`, re-derived every build. A
    /// person's fold is `cause = repair` and is replayed; the two must not read the same.
    pub merged: usize,
    /// Pairs that looked like one thing and the shared-illumination rule refused.
    pub refused: usize,
    /// Pairs whose sides had already been resolved by some other path.
    pub moot: usize,
    /// Pairs that share an identity and do not look like one thing.
    pub undecided: usize,
}

/// Fold the candidates that look like one thing, and leave the rest as questions.
///
/// `conn` must be the store's own connection: the batch is read through it. `scoring` is the store's
/// policy, passed in rather than read from a facade.
pub async fn merge(
    conn: &DatabaseConnection,
    graph: &KnowledgeGraph,
    scoring: &Scoring,
) -> anyhow::Result<MergeReport> {
    let mut report = MergeReport::default();
    for (a, b) in candidates(conn).await? {
        let (Ok(left), Ok(right)) = (graph.get(a).await, graph.get(b).await) else {
            // One side no longer names a row; the next scan will not raise the pair at all.
            report.moot += 1;
            continue;
        };
        // A tombstone on either side means some other path already answered this.
        if left.merged_into_id.is_some() || right.merged_into_id.is_some() {
            report.moot += 1;
            continue;
        }

        // The more substantiated side survives; ties go to the lower id.
        let (winner, loser) = if right.confidence.total_cmp(&left.confidence) == Ordering::Greater {
            (right, left)
        } else {
            (left, right)
        };
        // A set of one is the pair question: is there a candidate that is this thing?
        if scoring
            .merge
            .rank(&loser, std::slice::from_ref(&winner))
            .is_empty()
        {
            report.undecided += 1;
            continue;
        }
        match graph
            .merge(loser.id, winner.id, Cause::ProximityAuto)
            .await?
        {
            Outcome::Merged => report.merged += 1,
            Outcome::AlreadyMerged => report.moot += 1,
            // A question, not an error: the pair stays in the batch for a person to answer.
            Outcome::RefusedSharedIllumination => report.refused += 1,
        }
    }
    Ok(report)
}

/// The batch, most likely first.
async fn candidates(conn: &DatabaseConnection) -> anyhow::Result<Vec<(i32, i32)>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT holder_id, counterpart_id \
               FROM knowledge_proximities \
              ORDER BY score DESC NULLS LAST, id",
        ))
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get_by_index(0)?, row.try_get_by_index(1)?)))
        .collect()
}
