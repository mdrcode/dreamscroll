//! The heuristic: every number the graph guesses, in one file.
//!
//! Two verbs, implemented for the pairs the graph asks about:
//!
//! - [`Similarity`] — the weighted total of `name`, `description` and type agreement.
//! - [`Rank`] — the same total, filtered by [`worth_merging`] and ordered best first.
//!
//! The filter is the part a number could not carry: `same` is a conjunction — a name match *and*
//! something corroborating it — which is why [`Rank`] exists rather than a threshold on the score.
//! A pair that fails the conjunction still reports its total, so "same name, different type: 0.74"
//! stays visible on the proximity list even though it will never be folded.
//!
//! Not here: the confidence scale for authority answers (that is the authority's own calibration,
//! in `resolve/wikidata/ranking.rs`), the aggregate over claims ([`combine`], arithmetic), and the
//! order between two substantiated things (`scoring::by_confidence`, a convention).

use std::collections::BTreeSet;

use crate::model;

use super::evidence::ClaimBasis;
use super::normalize;
use super::scoring::{Rank, Scoring, Similarity, Subject};

pub const WEIGHT_NAME: f32 = 0.60;
pub const WEIGHT_DESCRIPTION: f32 = 0.30;
/// Deliberately small: with both types model-assigned, disagreement says more about the extractor
/// than about the referent (111 of 179 prod collisions are the same thing typed twice).
pub const WEIGHT_TYPE: f32 = 0.10;

/// The bar for a provisional → provisional merge, which is the only regime that exists until
/// resolution gives us identifiers.
pub(crate) const MERGE_BAR: f32 = 0.60;

/// A merge without an identifier needs a name match to be in the running at all.
const NAME_FLOOR: f32 = 0.90;

/// …and the anchor must agree about *something else* as well.
const AGREEMENT_FLOOR: f32 = 0.50;

/// Same name once word order and articles are folded.
const NAME_TOKEN_MATCH: f32 = 0.9;

/// Two types agreeing when one is `unknown`: an absence of information, not a disagreement.
const TYPE_UNKNOWN_MATCH: f32 = 0.5;

/// What an extra independent input is worth on top of the best claim's score.
pub const CORROBORATION_BONUS: f32 = 0.1;
pub const CONFIDENCE_CEILING: f32 = 1.0;

/// The heuristic policy, as one value. Stateless: every number is a constant above.
#[derive(Debug, Default, Clone, Copy)]
pub struct Heuristic;

impl Heuristic {
    /// This policy in the shape a store holds: the three questions, boxed.
    pub(super) fn scoring() -> Scoring {
        Scoring::new(std::sync::Arc::new(Self))
    }
}

impl Similarity<model::knowledge_entity::Model, model::knowledge_entity::Model> for Heuristic {
    fn score(
        &self,
        a: &model::knowledge_entity::Model,
        b: &model::knowledge_entity::Model,
    ) -> f32 {
        features(&Subject::of_entity(a), b).total
    }
}

impl Similarity<Subject, model::knowledge_entity::Model> for Heuristic {
    fn score(&self, subject: &Subject, b: &model::knowledge_entity::Model) -> f32 {
        features(subject, b).total
    }
}

impl Rank<Subject, model::knowledge_entity::Model> for Heuristic {
    fn rank(
        &self,
        subject: &Subject,
        candidates: &[model::knowledge_entity::Model],
    ) -> Vec<(usize, f32)> {
        let mut scored: Vec<(usize, f32)> = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let features = features(subject, candidate);
                worth_merging(&features).then_some((index, features.total))
            })
            .collect();
        // Best total first; ties keep the candidate order, which the blocker hands back by id.
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
    }
}

impl Rank<model::knowledge_entity::Model, model::knowledge_entity::Model> for Heuristic {
    fn rank(
        &self,
        a: &model::knowledge_entity::Model,
        candidates: &[model::knowledge_entity::Model],
    ) -> Vec<(usize, f32)> {
        self.rank(&Subject::of_entity(a), candidates)
    }
}

/// What one candidate earns against one subject, per feature and in total.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Features {
    pub name: f32,
    pub description: f32,
    pub type_match: f32,
    pub total: f32,
}

/// Whether the features justify calling two things one.
///
/// A weight alone is not enough: the observed Django failure is one exact name, two referents, and
/// a description sharing only enough tokens to lift the total over the bar. Without an identifier, a
/// name match has to be corroborated by the type or the text — and that conjunction is why this is
/// a filter on a vector of components rather than a threshold on its sum.
fn worth_merging(features: &Features) -> bool {
    features.name >= NAME_FLOOR
        && features.total >= MERGE_BAR
        && (features.type_match >= AGREEMENT_FLOOR || features.description >= AGREEMENT_FLOOR)
}

/// How substantiated an entity is: its best claim's score, plus a bonus per extra independent input
/// (`text` and `handle` are the two). `None` when it holds no claim.
///
/// Aggregating by "best + corroboration" rather than a sum is deliberate: two weak confirmations of
/// the same fact must not outrank one strong one.
pub(super) fn combine(claims: &[(ClaimBasis, Option<f32>)]) -> Option<f32> {
    if claims.is_empty() {
        return None;
    }
    let bases: BTreeSet<&str> = claims.iter().map(|claim| claim.0.as_str()).collect();
    let best = claims
        .iter()
        .map(|claim| claim.1.unwrap_or(0.0))
        .fold(0.0f32, f32::max);
    let extra = bases.len().saturating_sub(1) as f32 * CORROBORATION_BONUS;
    Some((best + extra).min(CONFIDENCE_CEILING))
}

/// The observation being compared, with its normalized forms precomputed.
#[derive(Debug, Clone)]
struct NormalizedObservation {
    name_key: String,
    token_key: String,
    description_tokens: Vec<String>,
    entity_type: String,
}

impl NormalizedObservation {
    fn new(name: &str, description: Option<&str>, entity_type: &str) -> Self {
        let name_key = normalize::comparable(name);
        let token_key = token_key_of(&tokens_of(&name_key));
        Self {
            token_key,
            name_key,
            description_tokens: description.map(tokens).unwrap_or_default(),
            entity_type: entity_type.to_string(),
        }
    }
}

/// A candidate entity, with the label work precomputed so scoring is cheap in a loop.
#[derive(Debug, Clone)]
struct NormalizedCandidate {
    alt_name_keys: Vec<String>,
    alt_name_token_keys: Vec<String>,
    /// One token set per accumulated description; a candidate has to fit only one of them.
    description_tokens: Vec<Vec<String>>,
    /// Every type this entity's members asserted. Agreement with any one of them counts.
    entity_types: Vec<String>,
}

impl NormalizedCandidate {
    /// `name` is the entity's selected surface and must be indexed alongside the alternatives.
    /// `project` deliberately excludes it from `alt_names`, so indexing `alt_names` alone leaves a
    /// persisted entity invisible to blocking on its own name — every later observation that names
    /// it then creates a duplicate instead of attaching.
    fn new(
        name: &str,
        alt_names: &[String],
        descriptions: &[&str],
        entity_types: &[String],
    ) -> Self {
        let mut alt_name_keys = Vec::with_capacity(alt_names.len() + 1);
        let mut alt_name_token_keys = Vec::with_capacity(alt_names.len() + 1);
        for surface in std::iter::once(name).chain(alt_names.iter().map(String::as_str)) {
            if let Some((key, blocking)) = block_keys(surface) {
                alt_name_keys.push(key);
                alt_name_token_keys.push(blocking);
            }
        }

        Self {
            alt_name_keys,
            alt_name_token_keys,
            description_tokens: descriptions
                .iter()
                .map(|description| tokens(description))
                .filter(|tokens| !tokens.is_empty())
                .collect(),
            entity_types: entity_types.to_vec(),
        }
    }
}

impl From<&model::knowledge_entity::Model> for NormalizedCandidate {
    fn from(entity: &model::knowledge_entity::Model) -> Self {
        let descriptions: Vec<&str> = entity
            .description
            .iter()
            .map(String::as_str)
            .chain(entity.alt_descriptions.iter().map(String::as_str))
            .collect();
        let types: Vec<String> = entity.types().map(str::to_string).collect();
        Self::new(
            &entity.name,
            &entity.alt_names,
            &descriptions,
            &types,
        )
    }
}

/// Similarity of one subject to one candidate: per feature, and weighted into a total.
fn features(subject: &Subject, candidate: &model::knowledge_entity::Model) -> Features {
    let observation = NormalizedObservation::new(
        &subject.name,
        subject.description.as_deref(),
        &subject.entity_type,
    );
    let candidate = NormalizedCandidate::from(candidate);
    score(&observation, &candidate)
}

/// Similarity of one normalized observation to one candidate: per feature, weighted into a total.
fn score(observation: &NormalizedObservation, candidate: &NormalizedCandidate) -> Features {
    let name = name_similarity(observation, candidate);
    // Best fit across every description the candidate has accumulated, never their union: a union
    // would dilute a good match with text from captures that had nothing to do with this observation.
    let description = candidate
        .description_tokens
        .iter()
        .map(|tokens| normalize::jaccard(&observation.description_tokens, tokens))
        .fold(0.0f32, f32::max);
    // Any agreement counts. An entity whose members were called both `brand` and `organization` must
    // accept either, or half its own members could not have joined it.
    let type_match = candidate
        .entity_types
        .iter()
        .map(|entity_type| type_similarity(&observation.entity_type, entity_type))
        .fold(0.0f32, f32::max);
    let total = WEIGHT_NAME * name + WEIGHT_DESCRIPTION * description + WEIGHT_TYPE * type_match;
    Features {
        name,
        description,
        type_match,
        total,
    }
}

/// 1.0 on the same normalized surface, 0.9 when only word order or an article differs, else 0.0.
/// A name never decides on its own — [`worth_merging`] demands corroboration.
fn name_similarity(observation: &NormalizedObservation, candidate: &NormalizedCandidate) -> f32 {
    if candidate.alt_name_keys.contains(&observation.name_key) {
        return 1.0;
    }
    if candidate
        .alt_name_token_keys
        .contains(&observation.token_key)
    {
        return NAME_TOKEN_MATCH;
    }
    0.0
}

/// 1.0 for the same type, `TYPE_UNKNOWN_MATCH` when either is unknown, else 0.
fn type_similarity(a: &str, b: &str) -> f32 {
    match (a, b) {
        (x, y) if x == y => 1.0,
        // An unknown type is an absence of information, not a disagreement.
        ("unknown", _) | (_, "unknown") => TYPE_UNKNOWN_MATCH,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------------------------
// The string plumbing the scoring and the blocker share. `normalize::comparable` is the core's;
// these are the guess's, and nothing outside `heuristic_scoring` and `name_blocker` asks for them.
// ---------------------------------------------------------------------------------------------

/// The two keys one surface answers to: its comparable form, and the order-insensitive token key.
/// `None` for a surface that folds to nothing.
pub(super) fn block_keys(surface: &str) -> Option<(String, String)> {
    let key = normalize::comparable(surface);
    if key.is_empty() {
        return None;
    }
    let blocking = token_key_of(&tokens_of(&key));
    Some((key, blocking))
}

/// Dropped from the *token* key only. They are noise for blocking, and never an identity decision.
const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// The words of a surface, in order, folded the same way `normalize::comparable` folds it.
fn tokens(surface: &str) -> Vec<String> {
    tokens_of(&normalize::comparable(surface))
}

/// The words of a surface already in its comparable form, so a caller that has one does not
/// normalize it twice.
fn tokens_of(comparable: &str) -> Vec<String> {
    comparable.split_whitespace().map(str::to_string).collect()
}

/// A word-order- and article-insensitive key, for blocking only. Never an identity key.
fn token_key_of(words: &[String]) -> String {
    let mut words: Vec<&str> = words.iter().map(String::as_str).collect();
    if words.len() > 1 && ARTICLES.contains(&words[0]) {
        words.remove(0);
    }
    words.sort_unstable();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::knowledge::scoring::{Similarity, Subject};
    use crate::test_support::fixtures;

    fn subject(name: &str, description: Option<&str>, ty: &str) -> Subject {
        let mut row = fixtures::stub_observation(1, name, ty);
        row.description = description.map(str::to_string);
        Subject::of_observation(&row)
    }

    fn entity(id: i32, name: &str, description: Option<&str>, ty: &str) -> model::knowledge_entity::Model {
        let mut row = fixtures::stub_entity(id, name, ty);
        row.description = description.map(str::to_string);
        row
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let mut candidate = fixtures::stub_entity(1, "Nintendo", "organization");
        candidate.alt_types = vec!["brand".to_string()];
        let typed = |ty: &str| features(&subject("Nintendo", None, ty), &candidate).type_match;
        assert_eq!(typed("brand"), 1.0);
        assert_eq!(typed("organization"), 1.0);
        assert_eq!(typed("movie"), 0.0);
    }

    #[test]
    fn identical_name_and_type_clears_the_bar() {
        let candidate = entity(1, "Train Dreams", Some("a novella by Denis Johnson"), "book");
        let subject = subject("Train Dreams", Some("a novella by Denis Johnson"), "book");
        let ranked = Heuristic.rank(&subject, std::slice::from_ref(&candidate));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, 0);
        assert!(ranked[0].1 > 0.99, "{ranked:?}");
    }

    #[test]
    fn article_or_word_order_alone_still_clears() {
        let candidate = entity(1, "Train Dreams", None, "book");
        let ranked = Heuristic.rank(&subject("The Train Dreams", None, "book"), &[candidate]);
        assert_eq!(ranked.len(), 1, "an article is not a different name: {ranked:?}");
        assert!(ranked[0].1 > MERGE_BAR, "{ranked:?}");
    }

    /// The pair the graph must never fold, and the number that says why: one exact name, no
    /// corroboration. The score clears [`MERGE_BAR`] on its own — which is exactly why the filter is
    /// a conjunction and not a threshold on the total.
    #[test]
    fn a_type_mismatch_without_name_agreement_does_not_clear() {
        let django = entity(1, "Django", Some("a 2021 video game"), "product");
        let frame = subject("Django", Some("a Python web framework"), "software");
        assert!(
            Heuristic.score(&frame, &django) >= MERGE_BAR,
            "the total alone would fold this pair"
        );
        assert!(Heuristic.rank(&frame, &[django]).is_empty());
    }

    #[test]
    fn different_names_do_not_clear() {
        let other = entity(1, "Gravity's Rainbow", None, "book");
        assert!(Heuristic
            .rank(&subject("Train Dreams", None, "book"), &[other])
            .is_empty());
    }

    #[test]
    fn unknown_type_is_weak_not_negative() {
        let candidate = entity(1, "Wojak", None, "concept");
        assert_eq!(features(&subject("Wojak", None, "unknown"), &candidate).type_match, 0.5);
    }

    /// The filter is a filter, not a reordering: the strongest candidate wins, and the ones that do
    /// not qualify are absent rather than last.
    #[test]
    fn rank_keeps_only_qualifying_candidates_best_first() {
        let weak = entity(1, "Train Dreams", None, "book");
        let strong = entity(2, "Train Dreams", Some("a novella by Denis Johnson"), "book");
        let unrelated = entity(3, "Dune", None, "book");
        let ranked = Heuristic.rank(
            &subject("Train Dreams", Some("a novella by Denis Johnson"), "book"),
            &[weak, strong, unrelated],
        );
        assert_eq!(ranked.len(), 2, "{ranked:?}");
        assert_eq!(ranked[0].0, 1, "the corroborated one leads: {ranked:?}");
        assert!(ranked.iter().all(|(index, _)| *index != 2));
    }

    #[test]
    fn token_key_ignores_order_and_articles() {
        let blocking = |surface: &str| block_keys(surface).expect("a surface").1;
        assert_eq!(blocking("The Train Dreams"), blocking("train dreams"));
        assert_eq!(blocking("Dreams Train"), blocking("Train Dreams"));
        assert_ne!(blocking("C"), blocking("C#"));
    }

    /// `None` is the least worth there is, and a tie is not a win for either side.
    #[test]
    fn by_confidence_puts_nothing_last() {
        use crate::knowledge::scoring::by_confidence;
        assert_eq!(by_confidence(None, Some(0.5)), std::cmp::Ordering::Less);
        assert_eq!(by_confidence(Some(0.5), None), std::cmp::Ordering::Greater);
        assert_eq!(by_confidence(Some(0.5), Some(0.5)), std::cmp::Ordering::Equal);
    }

    /// Corroboration is a bonus on the best claim, not a sum.
    #[test]
    fn combine_adds_one_bonus_per_extra_basis() {
        let text = (ClaimBasis::Text, Some(0.8));
        let handle = (ClaimBasis::Handle, Some(0.9));
        assert_eq!(combine(&[]), None);
        assert_eq!(combine(&[text]), Some(0.8));
        assert_eq!(combine(&[text, handle]), Some(0.9 + CORROBORATION_BONUS));
        // The same basis twice is not a second input.
        assert_eq!(combine(&[text, text]), Some(0.8));
        // …and the total is capped.
        assert_eq!(
            combine(&[(ClaimBasis::Text, Some(1.0)), handle]),
            Some(CONFIDENCE_CEILING)
        );
    }

    /// The subject carries the whole row; this policy reads three fields of it. A handle, the name it
    /// displayed, or the illumination's hint must not move the score: identity is settled *before*
    /// measuring, and folding it into the name is how a handle and a real name become two entities.
    #[test]
    fn the_subject_carries_more_than_the_score_reads() {
        let plain = fixtures::stub_observation(1, "Alex Lei", "real_person");
        let mut present = plain.clone();
        present.platform = Some("x_twitter".to_string());
        present.platform_handle = Some("@alexL_E_I".to_string());
        present.platform_display_name = Some("alex lei".to_string());
        present.search_query = Some("Alex Lei photographer".to_string());

        let candidate = fixtures::stub_entity(9, "Alex Lei", "real_person");
        assert_eq!(
            Heuristic.score(&Subject::of_observation(&plain), &candidate),
            Heuristic.score(&Subject::of_observation(&present), &candidate),
        );
    }
}
