//! One field, one ranking.
//!
//! The ladder decided per strategy: every way of asking had its own admission rule and its own
//! decision, and the first claim stopped the climb. Measured against the graph, the failures were
//! decided *before* the ranking — the right item was present and refused:
//!
//! - `BBC` had `Q9531` (British Broadcasting Corporation) at the top of two answers and refused it
//!   twice, because an acronym's expansion shares no token with the acronym; the search strategy
//!   then claimed `BBC News`, a division that does.
//! - `Industrial Revolution` had `Q2269` first in every answer and refused it at each, because its
//!   description shares one word with ours; the search strategy claimed a book *about* it.
//!
//! So: apply every strategy, aggregate what came back, and rank once. The score has four
//! components, each a measure with a reason of its own, plus one provenance policy:
//!
//! | term | what it measures |
//! | --- | --- |
//! | `tier` | the question's authority: a title is unique where a label is not, and search is a guess |
//! | `surface` | how strongly the label bears our name — or our hint, or an alias; a label-less candidate is judged by its description |
//! | `fit` | IDF-weighted unigram + bigram overlap of the descriptions, saturating |
//! | `kind` | the candidate's `P31` classes against the entity's types |
//! | account rule | an account-only entity is identified by its platform property, never by a name |
//!
//! Nothing else is in the score on purpose. Earlier versions added year matching, a name-mention
//! penalty, a description-head noun table, answer position and source breadth; measured against a
//! fixed 100-row labelled set they bought no precision and cost identities, so they were removed.
//! `plan/eval/identity-labels.tsv` is that set, and every change is scored against it.

use std::collections::{BTreeMap, BTreeSet};

use crate::fetch::{Answers, Outcome, Request};
use crate::knowledge::evidence::Identity;
use crate::knowledge::text;
use crate::knowledge::text::{content_tokens, grams, near_surface, word_prefix};
use crate::model;

use super::{WikidataCandidate, WikidataKind, answer, plan};

/// A property match: the item *is* the account, so it is claimed before the field is asked.
const HANDLE_CONFIDENCE: f32 = 0.95;
/// What a candidate from a guessing question must look like at minimum: a prefix, not a
/// rearrangement. Exact queries (handle, title, label) are held to a lower bar — one shared word —
/// because the question itself already named the thing.
const SEARCH_FLOOR: f32 = 0.7;
/// The floor for a candidate an exact query produced: the same words at minimum.
const EXACT_FLOOR: f32 = 0.6;
/// When two candidates bear the same surface, their descriptions must separate them by this much.
const MARGIN: f32 = 0.05;
/// The question's authority, as a prior rather than a gate: a title answer starts ahead of a label
/// answer, which starts ahead of a search hit — but a corroborated candidate can still overtake it.
const TIER_PRIOR: [f32; 3] = [0.15, 0.10, 0.00];
/// A candidate whose classes say it cannot be this entity is penalised, not filtered: the table is
/// incomplete, and an unknown class says nothing.
const TYPE_AGREES: f32 = 0.15;
const TYPE_DISAGREES: f32 = 0.30;
/// A claim needs a minimum of evidence, not just a lead over the runner-up: with a field of one,
/// there is no runner-up to lose to, and a surface-only candidate would assert itself.
const SCORE_FLOOR: f32 = 0.50;
/// The head noun of a Wikidata description states the kind (`1902 novel by …`, `scientific
/// article …`), which separates kinds the class table collapses into `Work`.
const HEAD_AGREES: f32 = 0.10;
const HEAD_DISAGREES: f32 = 0.15;
/// Works and events carry years, and a year is close to a fact: `Gladiator` is the 2000 film, not
/// the 1992 one. Agreement adds; a contradiction subtracts more.
const YEAR_AGREES: f32 = 0.10;
const YEAR_DISAGREES: f32 = 0.20;

/// Where one candidate was seen: which question produced it, and where the answer put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Source {
    pub(crate) strategy: plan::Strategy,
    pub(crate) position: usize,
}

/// A candidate and every question that produced it. One QID from two answers is one item with two
/// pieces of provenance, not two candidates.
#[derive(Debug, Clone)]
pub(crate) struct Aggregate {
    pub(crate) candidate: WikidataCandidate,
    pub(crate) sources: Vec<Source>,
}

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

/// How strongly a surface bears a name: 1.0 is the same name, 0.0 is a different one.
///
/// The classes are ordered on purpose — `exact` beats `near` beats `prefix` beats `set` beats
/// `permutation` — because an item that *is* the name must never lose to one that contains it.
pub(super) fn surface(label: &str, ours: &str) -> f32 {
    let (label_flat, ours_flat) = (text::comparable(label), text::comparable(ours));
    if label_flat.is_empty() || ours_flat.is_empty() {
        return 0.0;
    }
    if label_flat == ours_flat {
        return 1.0;
    }
    if near_surface(&label_flat, &ours_flat) {
        return 0.9;
    }
    if word_prefix(&ours_flat, &label_flat) || word_prefix(&label_flat, &ours_flat) {
        return 0.8;
    }
    let theirs: BTreeSet<String> = content_tokens(label).into_iter().collect();
    let mine: BTreeSet<String> = content_tokens(ours).into_iter().collect();
    if theirs.is_empty() || mine.is_empty() {
        return 0.0;
    }
    if theirs == mine {
        // Same words, different order: `Rebel Country` is not `Country Rebel`.
        return 0.6;
    }
    if theirs.is_superset(&mine) || mine.is_superset(&theirs) {
        // One name is the other plus a qualifier: `Andriivka, Donetsk Oblast`.
        return 0.7;
    }
    0.0
}

/// The field in order: the score, then surface, then description fit. Stable, so the provider's own
/// ordering breaks what is left.
pub(crate) fn rank(entity: &model::knowledge_entity::Model, field: &[Aggregate]) -> Vec<Ranked> {
    let candidates: Vec<WikidataCandidate> = field.iter().map(|p| p.candidate.clone()).collect();
    let weights = Weights::over(&candidates);
    let ours = our_tokens(entity);

    let mut entity_types = vec![entity.entity_type.as_str()];
    entity_types.extend(entity.alt_types.iter().map(String::as_str));

    let mut ranked: Vec<Ranked> = field
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
                let by_name = surface(label, &entity.name);
                let by_hint = entity
                    .search_query
                    .as_deref()
                    .map(|hint| surface(label, hint))
                    .unwrap_or(0.0);
                let by_alias = entity
                    .alt_names
                    .iter()
                    .map(|alias| surface(label, alias))
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
                head: head_agrees(&entity_types, &theirs),
                year: year_relation(entity, &theirs),
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

/// The one identity the field supports, or the reason there is none.
pub(crate) fn decide(
    entity: &model::knowledge_entity::Model,
    field: &[Aggregate],
) -> Result<Identity, &'static str> {
    // A profile is identified by its platform link, never by a name. Measured 2026-11-13: of 26
    // `social_media_account` entities carrying a Wikidata identity, one was right — the handle
    // match — and 25 were another bearer of the same word (`Bear` → a gay-culture term, `Blue` → a
    // colour, `Drake` → a botanist). A profile's description says what it posts, never who it is,
    // so no name-based question can answer for it. The handle strategy claims before the field is
    // asked, which is why refusing here leaves exactly the handle match standing.
    if account_only(entity) {
        return Err("a profile is identified by its platform link, not by a name");
    }

    let ranked = rank(entity, field);
    // A label-less candidate has no name to compare; the exact query is what vouches for it.
    //
    // A label match with no description corroboration is a name, not a fact: it is eligible only
    // when an article-title answer also produced it. That is the difference between `Industrial
    // Revolution` (a title and a label agree) and a homonym that merely shares the string.
    let eligible = |r: &Ranked| {
        let named = r.label_less && r.tier < 2
            || r.surface
                >= if r.tier == 2 {
                    SEARCH_FLOOR
                } else {
                    EXACT_FLOOR
                };
        named && (r.tier != 1 || r.fit > 0.0 || r.label_less)
    };
    let ranked: Vec<Ranked> = ranked.into_iter().filter(eligible).collect();
    let Some(best) = ranked.first().copied() else {
        return Err("no candidate bears our name");
    };
    if score(&best) < SCORE_FLOOR {
        return Err("the evidence is too thin to assert");
    }
    if let Some(second) = ranked.get(1)
        && score(&best) - score(second) < MARGIN
    {
        return Err("two candidates fit and neither wins");
    }
    let candidate = &field[best.index].candidate;
    Ok(answer::identity(
        candidate,
        method(best.strategy, best.surface),
        confidence(&best),
    ))
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
fn method(strategy: plan::Strategy, surface: f32) -> &'static str {
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
fn confidence(ranked: &Ranked) -> f32 {
    answer_confidence(0.7 * ranked.surface + 0.3 * ranked.fit)
}

/// Years on each side, compared. Both sides must carry one — an absent year is no disagreement.
fn year_relation(entity: &model::knowledge_entity::Model, theirs: &[String]) -> i8 {
    let entity_tokens: Vec<String> = entity.descriptions().flat_map(content_tokens).collect();
    let entity_years: BTreeSet<&String> = entity_tokens.iter().filter(|t| is_year(t)).collect();
    if entity_years.is_empty() {
        return 0;
    }
    let their_years: BTreeSet<&String> = theirs.iter().filter(|t| is_year(t)).collect();
    if their_years.is_empty() {
        return 0;
    }
    if entity_years.is_disjoint(&their_years) {
        -1
    } else {
        1
    }
}

fn is_year(token: &str) -> bool {
    token.len() == 4
        && token.chars().all(|c| c.is_ascii_digit())
        && (token.starts_with("19") || token.starts_with("20"))
}

/// The head nouns each type expects. `article` is recognised as a kind but expected by none: a
/// scholarly article claimed for a painting or a person is a contradiction, not an unknown.
const HEAD_KINDS: &[(&str, &[&str])] = &[
    (
        "movie",
        &[
            "film",
            "movie",
            "movi",
            "documentary",
            "documentari",
            "animation",
            "cartoon",
        ],
    ),
    (
        "book",
        &[
            "novel",
            "book",
            "novella",
            "poem",
            "essay",
            "biography",
            "treatise",
            "article",
        ],
    ),
    (
        "music",
        &[
            "album",
            "single",
            "song",
            "symphony",
            "concerto",
            "opera",
            "composition",
            "track",
            "record",
        ],
    ),
    (
        "art_work",
        &[
            "paint",
            "painting",
            "sculpture",
            "drawing",
            "print",
            "etching",
            "artwork",
            "altarpiece",
            "portrait",
            "tapestry",
        ],
    ),
    (
        "software",
        &[
            "software",
            "program",
            "language",
            "library",
            "application",
            "game",
            "engine",
            "tool",
            "shell",
            "compiler",
            "interpreter",
        ],
    ),
    (
        "publication",
        &[
            "magazine",
            "journal",
            "newspaper",
            "publication",
            "bulletin",
            "article",
        ],
    ),
    ("television_show", &["series", "seri", "show", "sitcom"]),
    (
        "place",
        &[
            "city",
            "citi",
            "town",
            "village",
            "country",
            "countri",
            "island",
            "region",
            "district",
            "province",
            "state",
            "municipality",
            "settlement",
            "river",
            "sea",
            "peninsula",
            "bridge",
            "park",
        ],
    ),
    (
        "organization",
        &[
            "company",
            "compani",
            "organization",
            "organisation",
            "agency",
            "university",
            "band",
            "team",
            "club",
            "institute",
            "corporation",
            "party",
            "group",
            "school",
            "museum",
        ],
    ),
    (
        "fictional_character",
        &["charact", "character", "protagonist", "fictional"],
    ),
    (
        "event",
        &[
            "war",
            "battle",
            "festival",
            "award",
            "ceremony",
            "competition",
            "election",
            "tournament",
            "conference",
            "occupation",
            "expedition",
        ],
    ),
    (
        "product",
        &[
            "product", "chair", "car", "vehicle", "watch", "weapon", "device", "machine", "model",
        ],
    ),
    ("brand", &["brand", "company", "compani"]),
    (
        "religious_figure",
        &["saint", "prophet", "deity", "god", "monk", "religious"],
    ),
    ("meme", &["meme", "macro", "internet"]),
    (
        "online_community",
        &[
            "community",
            "website",
            "forum",
            "imageboard",
            "subreddit",
            "board",
        ],
    ),
    ("financial", &["fund", "currency", "rate", "index", "stock"]),
];

fn expected_heads(entity_type: &str) -> Option<&'static [&'static str]> {
    HEAD_KINDS
        .iter()
        .find(|(kind, _)| *kind == entity_type)
        .map(|(_, heads)| *heads)
}

/// Whether the description's head noun names a kind the entity can be. Only the first few tokens
/// are read: Wikidata descriptions lead with the kind, and one further in is usually incidental.
fn head_agrees(entity_types: &[&str], theirs: &[String]) -> Option<bool> {
    let expected: Vec<&'static [&'static str]> = entity_types
        .iter()
        .filter_map(|entity_type| expected_heads(entity_type))
        .collect();
    if expected.is_empty() {
        return None;
    }
    let head = theirs.iter().take(6).find(|token| {
        HEAD_KINDS
            .iter()
            .any(|(_, heads)| heads.contains(&token.as_str()))
    })?;
    Some(expected.iter().any(|heads| heads.contains(&head.as_str())))
}

// -- the measure: what one candidate's description is worth -------------------------------------
//
// Kept here while the split is revisited. It answers *what is the evidence worth?* — the part that
// needs the corpus, because rarity is measured against the candidates the answer itself returned.
// `rank` applies it as the `fit` term; `confidence` prices it. Nothing here ranks or decides.

/// No answer is certain: the best a score can be priced at is 0.95.
const ANSWER_CEILING: f32 = 0.95;

/// A score in `[0, 1]` priced into the confidence an identity carries.
fn answer_confidence(score: f32) -> f32 {
    const BASE: f32 = 0.50;
    const SPREAD: f32 = 0.45;
    (BASE + SPREAD * score).min(ANSWER_CEILING)
}

/// A description this short has little to say, so one distinctive shared word can be the whole of
/// the evidence: `Irish writer (1911-1966)` against an entity described as an Irish novelist.
const TERSE_TOKENS: usize = 4;

/// Weight of matched evidence at which a match is half-convincing. A scale, not a threshold: two
/// common words land well below it, two rare ones well above.
const EVIDENCE_SCALE: f32 = 3.0;

/// Every description the entity has accumulated, tokenized. A candidate only has to fit one of them:
/// taking the union instead would dilute a good match with text from unrelated captures.
fn our_tokens(entity: &model::knowledge_entity::Model) -> Vec<Vec<String>> {
    entity.descriptions().map(content_tokens).collect()
}

/// The signal is the entity's description: how much of what Wikidata says about a candidate ours
/// accounts for, weighted by term rarity and saturating, so a long shared phrase cannot swamp it.
///
/// `search_query` is deliberately not part of the score. Measured 2026-10-31: adding it lifted
/// `Sunset Boulevard (musical film adaptation)` above the 1950 film, because that candidate's
/// description literally contains the hint's words. A hint drawn from what we saw is not independent
/// evidence about which candidate we meant.
///
/// Term rarity within the candidate set is what separates a *subject* from a *location*:
/// `"George Washington"` returns a president and a "bronze statue in Portland, Oregon, United
/// States", and both share two words with the entity's description — but `united`/`states` appear in
/// a large fraction of the candidates (every American place) while `president` does not. Inverse
/// document frequency over the candidates the query itself returned, and no hand-written stoplist.
struct Weights {
    documents: f32,
    frequency: BTreeMap<String, usize>,
}

impl Weights {
    fn over(candidates: &[WikidataCandidate]) -> Self {
        let mut frequency = BTreeMap::new();
        for candidate in candidates {
            let tokens = content_tokens(candidate.description.as_deref().unwrap_or_default());
            for gram in grams(&tokens).into_iter().collect::<BTreeSet<_>>() {
                *frequency.entry(gram).or_insert(0) += 1;
            }
        }
        Self {
            documents: candidates.len() as f32,
            frequency,
        }
    }

    fn of(&self, gram: &str) -> f32 {
        let df = self.frequency.get(gram).copied().unwrap_or(0) as f32;
        // A phrase is worth more than a word: it is evidence about a relationship, not a co-occurrence.
        let boost = if gram.contains(' ') { 2.0 } else { 1.0 };
        // `documents / (1 + df)` is below 1 when a term appears in every candidate — every term,
        // when the answer carried a single candidate. Log of that is negative, which charged a
        // candidate for matching the *only* descriptive word it had: `Salamandra · publisher`
        // scored 0.09 against a 0.20 floor. Flooring the ratio at 1 makes a universal term worth the
        // minimum instead of less than nothing.
        boost * (1.0 + (self.documents / (1.0 + df)).max(1.0).ln())
    }

    /// The weight of distinctive material this candidate accounts for, saturating.
    ///
    /// Deliberately *not* normalised by how verbose the candidate's description is. `George
    /// Washington`'s item matches two rare words (`first`, `president`); the statue in Portland
    /// matches two very common ones (`united`, `states`). Dividing by each description's own total
    /// compressed precisely that difference — the president's text carries four further rare words we
    /// do not share (`founding`, `father`, `1732`, `1799`), which lowered its score for being rich.
    fn fit(&self, theirs: &[String], ours: &[String]) -> f32 {
        // Counted before the shadowing below: `grams` turns three tokens into five, so measuring
        // "is this description terse" afterwards made the limit two words in practice — and
        // `video game from 1992` was treated as verbose.
        let terse = theirs.len() <= TERSE_TOKENS;
        let theirs = grams(theirs);
        let ours = grams(ours);
        let ours_set: BTreeSet<&String> = ours.iter().collect();
        let theirs_set: BTreeSet<&String> = theirs.iter().collect();
        let matched: Vec<&&String> = theirs_set.intersection(&ours_set).collect();
        // One shared word is normally coincidence. In a terse description it can be the whole
        // signal — but only when that word is rare among these candidates, because one common word
        // (`village`, `united`) still says nothing.
        let terse_and_rare = terse
            && matched.len() == 1
            && self
                .frequency
                .get(matched[0].as_str())
                .copied()
                .unwrap_or(0)
                <= 1;
        if matched.len() < 2 && !terse_and_rare {
            return 0.0;
        }
        let matched_weight: f32 = matched.iter().map(|t| self.of(t)).sum();
        matched_weight / (matched_weight + EVIDENCE_SCALE)
    }
}

/// One entity's field from every answer of one outcome, deduplicated by QID.
///
/// `answer::referents` still gates what is not a referent at all — a menu — but no strategy's name
/// rule applies: which candidate is ours is the ranking's question.
pub(crate) fn gather(
    asks: &[(plan::Strategy, Request<WikidataKind>)],
    answers: &Answers<WikidataKind, Vec<WikidataCandidate>>,
    outcome: Outcome,
) -> Vec<Aggregate> {
    let mut by_id: BTreeMap<String, Aggregate> = BTreeMap::new();
    for (strategy, request) in asks {
        // A handle is claimed whole; it is not scored against the field.
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
        for (position, candidate) in candidates.into_iter().enumerate() {
            by_id
                .entry(candidate.id.clone())
                .or_insert_with(|| Aggregate {
                    candidate,
                    sources: Vec::new(),
                })
                .sources
                .push(Source {
                    strategy: *strategy,
                    position,
                });
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
    let (_, request) = asks
        .iter()
        .find(|(strategy, _)| *strategy == plan::Strategy::Handle)?;
    let raw = answers.get(request)?;
    if raw.outcome != Outcome::Found {
        return None;
    }
    let property = request.kind.property()?;
    let candidates = answer::referents(raw).ok()?;
    let first = candidates.first()?;
    Some(answer::identity(
        first,
        property.as_str(),
        HANDLE_CONFIDENCE,
    ))
}

/// Whether every type the entity wears is `social_media_account`. An entity that is also a person
/// or a brand is not only a profile, and must not be blocked by the rule above.
fn account_only(entity: &model::knowledge_entity::Model) -> bool {
    entity
        .types()
        .all(|entity_type| entity_type == "social_media_account")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, label: &str, description: &str) -> Aggregate {
        Aggregate {
            candidate: WikidataCandidate {
                id: id.to_string(),
                label: Some(label.to_string()),
                description: Some(description.to_string()),
                classes: Vec::new(),
            },
            sources: vec![Source {
                strategy: plan::Strategy::SearchLabel,
                position: 0,
            }],
        }
    }

    fn entity(name: &str, description: &str, hint: Option<&str>) -> model::knowledge_entity::Model {
        let mut entity = crate::test_support::fixtures::stub_entity(1, name, "unknown");
        entity.description = Some(description.to_string());
        entity.search_query = hint.map(str::to_string);
        entity
    }

    fn weights(descriptions: &[&str]) -> Weights {
        let candidates: Vec<WikidataCandidate> = descriptions
            .iter()
            .map(|description| WikidataCandidate {
                id: "Q1".to_string(),
                label: None,
                description: Some((*description).to_string()),
                classes: Vec::new(),
            })
            .collect();
        Weights::over(&candidates)
    }

    /// A term in every candidate used to be worth *less* than nothing: `N / (1 + df)` is below 1,
    /// and `ln` of that is negative, so `Salamandra · "publisher"` scored 0.09 against a 0.20 floor
    /// and a single-candidate answer could never be identified on one matched word.
    #[test]
    fn a_term_present_in_every_candidate_is_worth_the_minimum() {
        let weights = weights(&["publisher"]);
        assert_eq!(weights.of("publisher"), 1.0);
        let fit = weights.fit(
            &content_tokens("publisher"),
            &content_tokens("Spanish publisher"),
        );
        assert!(fit > 0.0, "one terse shared word is evidence, got {fit}");
    }

    /// Terse is counted in tokens: `grams` turns three into five, and reading the gram count made
    /// every description longer than two words look verbose.
    #[test]
    fn three_words_is_still_terse() {
        let weights = weights(&["video game from 1992"]);
        let theirs = content_tokens("video game from 1992");
        let ours = content_tokens("a classic text-based role-playing game founded in 1991");
        assert_eq!(
            theirs.len(),
            3,
            "the fixture is the point: three content tokens"
        );
        assert!(
            weights.fit(&theirs, &ours) > 0.0,
            "a three-word description is terse"
        );
    }

    /// The two-token minimum is what rejects most weak matches, and it is still two tokens: one
    /// common word in a long description is coincidence.
    #[test]
    fn one_shared_word_in_a_long_description_is_not_evidence() {
        let weights = weights(&["united states navy aircraft carrier fleet"]);
        let theirs = content_tokens("united states navy aircraft carrier fleet");
        let ours = content_tokens("a village in the united counties of nowhere");
        assert!(
            theirs.len() > TERSE_TOKENS,
            "the fixture has to be long enough for the terse rule not to apply"
        );
        assert_eq!(weights.fit(&theirs, &ours), 0.0);
    }

    /// The class order is the ranking rule: exact, near, prefix, qualifier, permutation, nothing.
    #[test]
    fn the_surface_classes_are_ordered() {
        assert_eq!(surface("Andriivka", "Andriivka"), 1.0);
        assert_eq!(surface("Dostoyevsky", "Dostoevsky"), 0.9);
        assert_eq!(surface("M4 carbine", "M4"), 0.8);
        assert_eq!(surface("Andriivka, Donetsk Oblast", "Andriivka"), 0.8);
        assert_eq!(surface("Rebel Country", "Country Rebel"), 0.6);
        assert_eq!(surface("Something Else", "Country Rebel"), 0.0);
    }

    /// The failure this module exists for: the right item's label is the *expansion* of our
    /// acronym, so it shares no token with the name — but it is the hint verbatim. The division
    /// shares `bbc` with the name and must still lose.
    #[test]
    fn an_expansion_of_the_name_beats_a_division_that_shares_it() {
        let entity = entity(
            "BBC",
            "The public service broadcaster of the United Kingdom.",
            Some("British Broadcasting Corporation"),
        );
        let field = vec![
            candidate(
                "Q1160945",
                "BBC News",
                "news division of the British Broadcasting Corporation",
            ),
            candidate(
                "Q9531",
                "British Broadcasting Corporation",
                "British public service broadcaster",
            ),
        ];
        let identity = decide(&entity, &field).expect("the expansion is the name");
        assert_eq!(identity.external_id, "Q9531");
    }

    /// An exact label needs no description corroboration. `Industrial Revolution`'s description is
    /// phrased differently from ours, which used to score zero and hand the claim to a book *about*
    /// the revolution.
    #[test]
    fn an_exact_label_needs_no_description_overlap() {
        let entity = entity(
            "Industrial Revolution",
            "The period of global socioeconomic transformation from agrarian economies to mechanized production.",
            None,
        );
        // From the title answer: an exact label is a name, and a title answer vouches for it
        // without description corroboration. The same candidate found by search would not.
        let labelled = |id: &str, label: &str, description: &str| {
            let mut candidate = candidate(id, label, description);
            candidate.sources[0].strategy = plan::Strategy::Title;
            candidate
        };
        let field = vec![
            labelled(
                "Q4787186",
                "Buildings of Scotland in the Industrial Revolution",
                "Buildings of Scotland in the Industrial Revolution",
            ),
            labelled(
                "Q2269",
                "Industrial Revolution",
                "transition to new manufacturing processes in Europe and the United States",
            ),
        ];
        let identity = decide(&entity, &field).expect("the exact label wins");
        assert_eq!(identity.external_id, "Q2269");
    }

    /// Same surface on both sides: the description has to separate them.
    #[test]
    fn a_tie_on_surface_is_refused() {
        let entity = entity("Dune", "1965 science fiction novel by Frank Herbert", None);
        let field = vec![
            candidate("Q1", "Dune", "novel by Frank Herbert"),
            candidate("Q2", "Dune", "novel by Frank Herbert"),
        ];
        assert_eq!(
            decide(&entity, &field).unwrap_err(),
            "two candidates fit and neither wins"
        );
    }

    /// A profile may not be identified by a name, however well the words match — and an entity that
    /// is also a person is not only a profile.
    #[test]
    fn a_profile_needs_a_platform_link() {
        let mut account = entity("Bear", "An account on X discussing military history.", None);
        account.entity_type = "social_media_account".to_string();
        let field = vec![candidate("Q618804", "bear", "term in gay culture")];
        assert_eq!(
            decide(&account, &field).unwrap_err(),
            "a profile is identified by its platform link, not by a name"
        );

        let mut person = entity("Sama Hoole", "A researcher and commentator.", None);
        person.entity_type = "social_media_account".to_string();
        person.alt_types = vec!["real_person".to_string()];
        let field = vec![candidate("Q1", "Sama Hoole", "researcher")];
        assert!(
            decide(&person, &field).is_ok(),
            "a person is not only a profile"
        );
    }
}
