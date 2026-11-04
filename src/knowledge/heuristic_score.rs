//! The heuristic: every number we guess, in one file.
//!
//! This module is the *entire* ad hoc policy behind [`super::score`]: what a name similarity is,
//! which weights apply, where the bars and margins sit, and what a claim is worth. The core exposes
//! the verbs; this file is one answer to all of them, and `Scoring::heuristic()` is how the rest of
//! the graph gets it.
//!
//! Three parts, mirroring the traits it implements:
//!
//! - [`Heuristic::fit`] — pure features: name, description, type, weighted into a total.
//! - [`Heuristic::same`], [`Heuristic::supported`], [`Heuristic::leads`] — the bars.
//! - [`Heuristic::value`], [`Heuristic::combine`], [`Heuristic::order`] — what evidence is worth.
//!
//! The normalization it needs — comparable forms, token keys, token sets — lives here too, because
//! it is this policy's idea of what "the same word" means. The blocking keys [`block_keys`] computes
//! are shared with [`super::name_blocker`], which is the other half of the same guess.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use crate::model;

use super::evidence::ClaimBasis;
use super::normalize;
use super::score::{Answer, Features, Judge, Similarity, Subject, Worth};

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

/// The least similarity that counts as evidence at all — an authority's answer, or a search hit,
/// is not worth asking about below this.
const EVIDENCE_FLOOR: f32 = 0.20;

/// The least lead that turns the best candidate into a decision. Two candidates that fit equally
/// well are ambiguity, not an answer.
const LEAD_MARGIN: f32 = 0.10;

/// What an extra independent input is worth on top of the best claim's score.
pub const CORROBORATION_BONUS: f32 = 0.1;
pub const CONFIDENCE_CEILING: f32 = 1.0;

/// A ranked authority answer, mapped into [0.5, `ANSWER_CEILING`].
const ANSWER_BASE: f32 = 0.50;
const ANSWER_SPREAD: f32 = 0.45;
pub const ANSWER_CEILING: f32 = 0.95;
/// A guess from the disambiguating words alone is worth less than an exact label.
pub const HINT_CEILING: f32 = 0.85;
/// One exact label with nothing competing for it is identity, but nothing corroborates it either.
pub const LONE_LABEL_CONFIDENCE: f32 = 0.75;
/// A search answer accepted because exactly one candidate's label is our own surface.
pub const LOOKUP_CONFIDENCE: f32 = 0.85;
/// An authority asked about a handle: a property match names the account itself.
pub const HANDLE_CONFIDENCE: f32 = 0.95;

/// The heuristic policy, as one value. Stateless: every number is a constant above.
#[derive(Debug, Default, Clone, Copy)]
pub struct Heuristic;

impl Similarity for Heuristic {
    fn fit(
        &self,
        subject: &Subject<'_>,
        candidates: &[model::knowledge_entity::Model],
    ) -> Vec<Features> {
        // The corpus is not consulted: this policy's weights are fixed, not learned from the set.
        // A policy that needed it (term rarity, say) has it by the argument.
        let view = NormalizedObservation::new(subject.name, subject.description, subject.entity_type);
        candidates
            .iter()
            .map(|candidate| score(&view, &NormalizedCandidate::from(candidate)))
            .collect()
    }
}

impl Judge for Heuristic {
    /// A weight alone is not enough: the observed Django failure is one exact name, two referents,
    /// and a description sharing only enough tokens to lift the total over the bar. Without an
    /// identifier, a name match has to be corroborated by the type or the text.
    fn same(&self, features: &Features) -> bool {
        features.name >= NAME_FLOOR
            && features.total >= MERGE_BAR
            && (features.type_match >= AGREEMENT_FLOOR || features.description >= AGREEMENT_FLOOR)
    }

    fn supported(&self, similarity: f32) -> bool {
        similarity >= EVIDENCE_FLOOR
    }

    fn leads(&self, gap: f32) -> bool {
        gap >= LEAD_MARGIN
    }
}

impl Worth for Heuristic {
    fn value(&self, answer: Answer, score: f32) -> f32 {
        match answer {
            Answer::Handle => HANDLE_CONFIDENCE,
            Answer::LoneLabel => LONE_LABEL_CONFIDENCE,
            Answer::Lookup => LOOKUP_CONFIDENCE,
            Answer::Hint => answer_confidence(score).min(HINT_CEILING),
            Answer::Ranked => answer_confidence(score),
        }
    }

    /// The best claim's score, plus a bonus per extra independent input (`text` and `handle` are the
    /// two). Aggregating by "best + corroboration" rather than a sum is deliberate: two weak
    /// confirmations of the same fact must not outrank one strong one.
    fn combine(&self, claims: &[(ClaimBasis, Option<f32>)]) -> Option<f32> {
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

    fn order(&self, a: Option<f32>, b: Option<f32>) -> Ordering {
        a.unwrap_or(f32::NEG_INFINITY)
            .total_cmp(&b.unwrap_or(f32::NEG_INFINITY))
    }
}

/// What a ranked authority answer is worth, before corroboration.
fn answer_confidence(score: f32) -> f32 {
    let mapped = ANSWER_BASE + ANSWER_SPREAD * score;
    mapped.min(ANSWER_CEILING)
}

/// The observation being linked, with its normalized forms precomputed.
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

/// Similarity of one observation to one candidate: per feature, and weighted into a total.
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
/// A name never decides on its own — [`Heuristic::same`] demands corroboration.
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
// these are the guess's, and nothing outside `heuristic_score` and `name_blocker` asks for them.
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

    fn observation(name: &str, description: Option<&str>, ty: &str) -> NormalizedObservation {
        NormalizedObservation::new(name, description, ty)
    }

    fn entity(name: &str, description: Option<&str>, ty: &str) -> NormalizedCandidate {
        let descriptions: Vec<&str> = description.into_iter().collect();
        NormalizedCandidate::new(name, &[], &descriptions, &[ty.to_string()])
    }

    fn features(name: &str, description: Option<&str>, ty: &str, other: &str) -> Features {
        score(&observation(name, description, ty), &entity(other, description, ty))
    }

    /// `brand` and `organization` are the same referent described twice — the taxonomy does not have
    /// to choose for a member to agree.
    #[test]
    fn any_asserted_type_counts_as_agreement() {
        let entity = NormalizedCandidate::new(
            "Nintendo",
            &[],
            &[],
            &["organization".to_string(), "brand".to_string()],
        );
        let typed = |ty: &str| score(&observation("Nintendo", None, ty), &entity).type_match;
        assert_eq!(typed("brand"), 1.0);
        assert_eq!(typed("organization"), 1.0);
        assert_eq!(typed("movie"), 0.0);
    }

    #[test]
    fn identical_name_and_type_clears_the_bar() {
        let features = features(
            "Train Dreams",
            Some("a novella by Denis Johnson"),
            "book",
            "Train Dreams",
        );
        assert_eq!(features.name, 1.0);
        assert!(Heuristic.same(&features), "{features:?}");
    }

    #[test]
    fn article_or_word_order_alone_still_clears() {
        let features = features("The Train Dreams", None, "book", "Train Dreams");
        assert_eq!(features.name, 0.9);
        assert!(Heuristic.same(&features), "{features:?}");
    }

    #[test]
    fn a_type_mismatch_without_name_agreement_does_not_clear() {
        let features = score(
            &observation("Django", Some("a Python web framework"), "software"),
            &NormalizedCandidate::new(
                "Django",
                &[],
                &["a 2021 video game"],
                &["product".to_string()],
            ),
        );
        assert_eq!(features.name, 1.0);
        assert_eq!(features.type_match, 0.0);
        // One exact name and a description that shares only stopword-ish tokens: this is the case
        // that made the resolver claim the video game, so it must not merge on weights alone.
        assert!(!Heuristic.same(&features), "{features:?}");
    }

    #[test]
    fn different_names_do_not_clear() {
        let features = features("Train Dreams", None, "book", "Gravity's Rainbow");
        assert_eq!(features.name, 0.0);
        assert!(!Heuristic.same(&features), "{features:?}");
    }

    #[test]
    fn unknown_type_is_weak_not_negative() {
        let features = score(
            &observation("Wojak", None, "unknown"),
            &entity("Wojak", None, "concept"),
        );
        assert_eq!(features.type_match, 0.5);
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
    fn order_puts_nothing_last() {
        assert_eq!(Heuristic.order(None, Some(0.5)), Ordering::Less);
        assert_eq!(Heuristic.order(Some(0.5), None), Ordering::Greater);
        assert_eq!(Heuristic.order(Some(0.5), Some(0.5)), Ordering::Equal);
    }

    /// Corroboration is a bonus on the best claim, not a sum: two weak confirmations of one fact
    /// must not outrank one strong one.
    #[test]
    fn combine_adds_one_bonus_per_extra_basis() {
        let text = (ClaimBasis::Text, Some(0.8));
        let handle = (ClaimBasis::Handle, Some(0.9));
        assert_eq!(Heuristic.combine(&[]), None);
        assert_eq!(Heuristic.combine(&[text]), Some(0.8));
        assert_eq!(
            Heuristic.combine(&[text, handle]),
            Some(0.9 + CORROBORATION_BONUS)
        );
        // The same basis twice is not a second input.
        assert_eq!(Heuristic.combine(&[text, text]), Some(0.8));
        // …and the total is capped.
        assert_eq!(
            Heuristic.combine(&[(ClaimBasis::Text, Some(1.0)), handle]),
            Some(CONFIDENCE_CEILING)
        );
    }

    #[test]
    fn a_handle_is_worth_more_than_a_ranked_answer() {
        assert_eq!(Heuristic.value(Answer::Handle, 0.0), HANDLE_CONFIDENCE);
        assert!(Heuristic.value(Answer::Ranked, 1.0) <= ANSWER_CEILING);
        assert!(Heuristic.value(Answer::Hint, 1.0) <= HINT_CEILING);
    }

    /// The subject carries the whole row; this policy reads three fields of it. A handle, the name it
    /// displayed, or the illumination's hint must not move the score: identity is settled *before*
    /// measuring, and folding it into the name is how a handle and a real name become two entities.
    #[test]
    fn the_subject_carries_more_than_the_score_reads() {
        use crate::knowledge::score::Subject;

        let mut plain =
            crate::test_support::fixtures::stub_observation(1, "Alex Lei", "real_person");
        let mut present = plain.clone();
        present.platform = Some("x_twitter".to_string());
        present.platform_handle = Some("@alexL_E_I".to_string());
        present.platform_display_name = Some("alex lei".to_string());
        present.search_query = Some("Alex Lei photographer".to_string());

        let candidate = crate::test_support::fixtures::stub_entity(9, "Alex Lei", "real_person");
        let one = std::slice::from_ref(&candidate);
        assert_eq!(
            Heuristic.fit(&Subject::of_observation(&plain), one),
            Heuristic.fit(&Subject::of_observation(&present), one),
        );
        plain.search_query = Some("ignored either way".to_string());
        assert_eq!(
            Heuristic.fit(&Subject::of_observation(&plain), one),
            Heuristic.fit(&Subject::of_observation(&present), one),
        );
    }
}
