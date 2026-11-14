//! The candidate set: what every answer offered, deduplicated, with who offered it.
//!
//! Admission without ranking. A strategy's answer contributes the items `answer::referents` accepts
//! — a menu is not a referent — and the same QID found twice is one entry with two sources, not two
//! candidates. The handle is the one exception: a property match *is* the identity, so it claims
//! before the set is built.

use std::collections::BTreeMap;

use crate::fetch::{Answers, Outcome, Request};
use crate::knowledge::evidence::Identity;

use super::answer;
use super::{WikidataCandidate, WikidataKind, plan};

/// A property match: the item *is* the account, so it is claimed before the set is asked.
const HANDLE_CONFIDENCE: f32 = 0.95;

/// A candidate and every strategy that produced it. One QID from two answers is one item with two
/// pieces of provenance, not two candidates. The strongest strategy is what `ranking` prices, and
/// all of them are what a diagnostic can show.
#[derive(Debug, Clone)]
pub(crate) struct Aggregate {
    pub(crate) candidate: WikidataCandidate,
    pub(crate) strategies: Vec<plan::Strategy>,
    /// The article-title question returned exactly this one item. A title question names one page,
    /// so this is the strongest surface a candidate can carry: the page title *is* our name even
    /// when the item's label is a synonym or a redirect target.
    pub(crate) title_only: bool,
}

/// Every candidate one entity's answers offered, deduplicated by QID, each carrying the strategies
/// that named it. `gather` builds it; `ranking` orders it; `decide` picks from it.
pub(crate) type CandidateSet = Vec<Aggregate>;

/// One entity's candidate set from every answer of one outcome, deduplicated by QID.
///
/// `answer::referents` still gates what is not a referent at all — a menu — but no strategy's name
/// rule applies: which candidate is ours is the ranking's question.
pub(crate) fn gather(
    asks: &[(plan::Strategy, Request<WikidataKind>)],
    answers: &Answers<WikidataKind, Vec<WikidataCandidate>>,
    outcome: Outcome,
) -> CandidateSet {
    let mut by_id: BTreeMap<String, Aggregate> = BTreeMap::new();
    for (strategy, request) in asks {
        // A handle is claimed whole; it is not scored against the set.
        if *strategy == plan::Strategy::Handle {
            continue;
        }
        let Some(raw) = answers.get(request) else {
            continue;
        };
        if raw.outcome != outcome {
            continue;
        }
        let Ok(candidates) = answer::referents(raw) else {
            continue;
        };
        let title_only = *strategy == plan::Strategy::Title && candidates.len() == 1;
        for candidate in candidates {
            let aggregate = by_id.entry(candidate.id.clone()).or_insert_with(|| Aggregate {
                candidate,
                strategies: Vec::new(),
                title_only: false,
            });
            aggregate.strategies.push(*strategy);
            if title_only {
                aggregate.title_only = true;
            }
        }
    }
    by_id.into_values().collect()
}

/// The handle answer, when there is one: the item asserting this property value is the account
/// itself, and no other candidate can outrank a property match.
pub(crate) fn by_handle(
    asks: &[(plan::Strategy, Request<WikidataKind>)],
    answers: &Answers<WikidataKind, Vec<WikidataCandidate>>,
) -> Option<Identity> {
    asks.iter()
        .filter(|(strategy, _)| *strategy == plan::Strategy::Handle)
        // One link may be asked in two spellings of one namespace (the canonical case and a
        // lowercase form), so the answer that carried an item is the one to take, not the first ask.
        .find_map(|(_, request)| {
            let raw = answers.get(request)?;
            if raw.outcome != Outcome::Found {
                return None;
            }
            let property = request.kind.property()?;
            let candidate = answer::referents(raw).ok()?.into_iter().next()?;
            Some(candidate.identity(property.as_str(), HANDLE_CONFIDENCE))
        })
}
