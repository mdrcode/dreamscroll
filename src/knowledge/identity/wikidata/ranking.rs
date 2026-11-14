//! The ranking: one row per candidate, ordered by what the measure and the authority say.
//!
//! This is the composition point. `measure` answers what each piece of evidence is worth; `score`
//! weighs those answers into one number, with the question's authority as a prior rather than a
//! gate. Nothing here admits or refuses: `decide` applies the eligibility floors and the margin to
//! the order this produces, and `gather` decided what was in the set at all.

use crate::knowledge::text::content_tokens;
use crate::model;

use super::gather::CandidateSet;
use super::measure::{self, Weights};
use super::{WikidataCandidate, plan};

/// The question's authority, as a prior rather than a gate: a title answer starts ahead of a label
/// answer, which starts ahead of a search hit — but a corroborated candidate can still overtake it.
const TIER_PRIOR: [f32; 3] = [0.15, 0.10, 0.00];
/// A candidate whose classes say it cannot be this entity is penalised, not filtered: the table is
/// incomplete, and an unknown class says nothing.
const TYPE_AGREES: f32 = 0.15;
const TYPE_DISAGREES: f32 = 0.30;
/// The head noun of a Wikidata description states the kind (`1902 novel by …`, `scientific
/// article …`), which separates kinds the class table collapses into `Work`.
const HEAD_AGREES: f32 = 0.10;
const HEAD_DISAGREES: f32 = 0.15;
/// Works and events carry years, and a year is close to a fact: `Gladiator` is the 2000 film, not
/// the 1992 one. Agreement adds; a contradiction subtracts more.
const YEAR_AGREES: f32 = 0.10;
const YEAR_DISAGREES: f32 = 0.20;

/// One candidate's place in the order, with the numbers that put it there.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ranked {
    pub(crate) index: usize,
    /// 0 for a property or article-title match, 1 for an exact label, 2 for a search hit. The
    /// question's authority outranks the surface: a title answer is about *this* name, a search
    /// answer merely mentions it.
    pub(crate) tier: u8,
    pub(crate) surface: f32,
    pub(crate) fit: f32,
    pub(crate) label_less: bool,
    /// Whether the candidate's classes can be what the entity is; `None` when none is recognised.
    pub(crate) kind: Option<bool>,
    /// Whether the description's head noun names a kind the entity can be; `None` when it states
    /// no recognised kind.
    pub(crate) head: Option<bool>,
    /// `1` when the descriptions agree on a year, `-1` when they carry disjoint years, `0` when
    /// either side has none.
    pub(crate) year: i8,
    pub(crate) strategy: plan::Strategy,
}

/// The candidate set in order: the score, then surface, then description fit. Stable, so the
/// provider's own ordering breaks what is left.
pub(crate) fn rank(
    entity: &model::knowledge_entity::Model,
    candidate_set: &CandidateSet,
) -> Vec<Ranked> {
    let offered: Vec<WikidataCandidate> = candidate_set
        .iter()
        .map(|aggregate| aggregate.candidate.clone())
        .collect();
    let weights = Weights::over(&offered);
    let ours = measure::our_tokens(entity);

    let mut entity_types = vec![entity.entity_type.as_str()];
    entity_types.extend(entity.alt_types.iter().map(String::as_str));

    let mut ranked: Vec<Ranked> = candidate_set
        .iter()
        .enumerate()
        .map(|(index, aggregate)| {
            let label = aggregate.candidate.label.as_deref().unwrap_or_default();
            let theirs = content_tokens(
                aggregate
                    .candidate
                    .description
                    .as_deref()
                    .unwrap_or_default(),
            );
            let fit = ours
                .iter()
                .map(|tokens| weights.fit(&theirs, tokens))
                .fold(0.0f32, f32::max);
            // A candidate with no label has no surface to bear; its description is the whole
            // evidence, and at `fit` 0.5 the description says as much as a name would. A labelled
            // one is judged against our name, the hint (an acronym's expansion, a rename) and any
            // alias the entity carries.
            let surface = if label.is_empty() {
                (2.0 * fit).min(1.0)
            } else {
                let by_name = measure::surface(label, &entity.name);
                let by_hint = entity
                    .search_query
                    .as_deref()
                    .map(|hint| measure::surface(label, hint))
                    .unwrap_or(0.0);
                let by_alias = entity
                    .alt_names
                    .iter()
                    .map(|alias| measure::surface(label, alias))
                    .fold(0.0f32, f32::max);
                by_name.max(by_hint).max(by_alias)
            };
            let (tier, strategy) = aggregate
                .sources
                .iter()
                .map(|source| (tier_of(source.strategy), source.strategy))
                .min_by_key(|(tier, _)| *tier)
                .unwrap_or((2, plan::Strategy::SearchLabel));
            Ranked {
                index,
                tier,
                surface,
                fit,
                label_less: label.is_empty(),
                kind: super::types::compatible(&entity_types, &aggregate.candidate.classes),
                head: measure::head_agrees(&entity_types, &theirs),
                year: measure::year_relation(entity, &theirs),
                strategy,
            }
        })
        .collect();

    ranked.sort_by(|a, b| {
        score(b)
            .total_cmp(&score(a))
            .then_with(|| b.surface.total_cmp(&a.surface))
            .then_with(|| b.fit.total_cmp(&a.fit))
    });
    ranked
}

/// The question's authority, the surface it bears, the description evidence, and the kind opinion.
pub(crate) fn score(ranked: &Ranked) -> f32 {
    let kind = match ranked.kind {
        Some(true) => TYPE_AGREES,
        Some(false) => -TYPE_DISAGREES,
        None => 0.0,
    };
    let head = match ranked.head {
        Some(true) => HEAD_AGREES,
        Some(false) => -HEAD_DISAGREES,
        None => 0.0,
    };
    let year = match ranked.year {
        1 => YEAR_AGREES,
        -1 => -YEAR_DISAGREES,
        _ => 0.0,
    };
    TIER_PRIOR[ranked.tier as usize]
        + 0.55 * ranked.surface
        + 0.50 * ranked.fit
        + kind
        + head
        + year
}

/// The authority of the question that produced a candidate: a handle is a property match, a title
/// is unique where a label is not, an exact label is a name, and search is a guess.
fn tier_of(strategy: plan::Strategy) -> u8 {
    match strategy {
        plan::Strategy::Handle | plan::Strategy::Title => 0,
        plan::Strategy::Label => 1,
        plan::Strategy::SearchHint | plan::Strategy::SearchLabel => 2,
    }
}

/// The strategy that produced the winner, in the vocabulary the rest of the system records.
pub(super) fn method(strategy: plan::Strategy, surface: f32) -> &'static str {
    match strategy {
        plan::Strategy::Handle => "handle",
        plan::Strategy::Title => "wikipedia_title",
        plan::Strategy::Label => "exact_label",
        // An exact surface found by search is the shortcut's old `search_lookup`.
        plan::Strategy::SearchHint | plan::Strategy::SearchLabel if surface >= 1.0 => {
            "search_lookup"
        }
        plan::Strategy::SearchHint | plan::Strategy::SearchLabel => "search_ranked",
    }
}

/// The score, priced into the confidence the identity carries.
pub(super) fn confidence(ranked: &Ranked) -> f32 {
    measure::answer_confidence(0.7 * ranked.surface + 0.3 * ranked.fit)
}
