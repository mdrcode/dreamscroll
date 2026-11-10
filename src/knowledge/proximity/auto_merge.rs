//! Auto-merge: fold the pairs that score as one thing on their own, leave the rest as questions.

use std::cmp::Ordering;

use sea_orm::sea_query::{NullOrdering, Order};
use sea_orm::{DatabaseConnection, EntityTrait, QueryOrder, QuerySelect};

use crate::knowledge::graph::KnowledgeGraph;
use crate::knowledge::graph::merge::Outcome;
use crate::knowledge::scoring::{Scoring, TotalCmp};
use crate::knowledge::verdicts::Cause;
use crate::model;

/// What one merge did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AutoMergeReport {
    /// Pairs this pass merged on its own — logged `proximity_auto`, re-derived every build. A
    /// person's fold is `cause = repair` and is replayed; the two must not read the same. In a dry
    /// run this is the count that *would* merge.
    pub merged: usize,
    /// Pairs that looked like one thing and the shared-illumination rule refused.
    pub refused: usize,
    /// Pairs whose sides had already been resolved by some other path.
    pub moot: usize,
    /// Pairs that share an identity and do not look like one thing.
    pub undecided: usize,
}

/// Fold the candidates that look like one thing on their own, and leave the rest as questions.
///
/// `apply` false is the preview: the same predicate, no writes. `conn` must be the store's own
/// connection. `scoring` is the store's policy, passed in rather than read from a facade.
pub async fn auto_merge(
    conn: &DatabaseConnection,
    graph: &KnowledgeGraph,
    scoring: &Scoring,
    apply: bool,
) -> anyhow::Result<AutoMergeReport> {
    let mut report = AutoMergeReport::default();
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
        if !apply {
            let preview = graph.merge_preview(loser.id, winner.id).await?;
            if preview.shared_illuminations > 0 {
                report.refused += 1;
            } else {
                report.merged += 1;
            }
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
    Ok(model::knowledge_proximities::Entity::find()
        .select_only()
        .columns([
            model::knowledge_proximities::Column::HolderId,
            model::knowledge_proximities::Column::CounterpartId,
        ])
        .order_by_with_nulls(
            model::knowledge_proximities::Column::Score,
            Order::Desc,
            NullOrdering::Last,
        )
        .order_by_asc(model::knowledge_proximities::Column::Id)
        .into_tuple::<(i32, i32)>()
        .all(conn)
        .await?)
}
