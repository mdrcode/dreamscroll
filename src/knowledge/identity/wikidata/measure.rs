//! The measure: what one candidate's evidence is worth, term by term.
//!
//! Nothing here ranks, weights or decides. Each function answers one question about a candidate —
//! how strongly its label bears our name, whether its description's head noun names a kind the
//! entity can be, whether the two descriptions agree on a year, how much our description accounts
//! for — and `ranking` prices the answers. The `fit` term is here rather than in `ranking` because
//! it needs the *corpus*: rarity is measured against the candidates the answer itself returned,
//! which a pair comparison cannot see.

use std::collections::{BTreeMap, BTreeSet};

use crate::knowledge::text::{self, content_tokens, grams, near_surface, word_prefix};
use crate::model;

use super::WikidataCandidate;

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

/// Years on each side, compared. Both sides must carry one — an absent year is no disagreement.
pub(super) fn year_relation(entity: &model::knowledge_entity::Model, theirs: &[String]) -> i8 {
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

/// A year this corpus cares about: four digits, in the range captures actually describe.
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
pub(super) fn head_agrees(entity_types: &[&str], theirs: &[String]) -> Option<bool> {
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

// -- the description evidence, and the pricing ------------------------------------------------

/// No answer is certain: the best a score can be priced at is 0.95.
const ANSWER_CEILING: f32 = 0.95;

/// A score in `[0, 1]` priced into the confidence an identity carries.
pub(super) fn answer_confidence(score: f32) -> f32 {
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
pub(super) fn our_tokens(entity: &model::knowledge_entity::Model) -> Vec<Vec<String>> {
    entity.descriptions().map(content_tokens).collect()
}

/// The description evidence for one candidate, given the corpus the answer supplied.
///
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
pub(super) struct Weights {
    documents: f32,
    frequency: BTreeMap<String, usize>,
}

impl Weights {
    pub(super) fn over(candidates: &[WikidataCandidate]) -> Self {
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
    pub(super) fn fit(&self, theirs: &[String], ours: &[String]) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
