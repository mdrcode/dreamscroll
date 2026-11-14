//! The measure: how well one candidate fits the entity, and how that orders.
//!
//! The decision — which candidate wins, or none — is `decision`'s. This module only produces the
//! evidence it decides on, so it is a [`Policy`] rather than a
//! [`Similarity`](crate::knowledge::scoring::Similarity): terms are weighed by rarity among the
//! answers the question returned, and a pair comparison cannot see that corpus.
//!
//! The signal is the entity's description, read two ways: *containment* — how much of what Wikidata
//! says about a candidate our description accounts for, which survives our description being long —
//! and symmetric *overlap*, which separates `Dune` (1965 novel) from `Dune` (the series), whose
//! description is a superset of ours.
//!
//! `search_query` is deliberately not part of the score. Measured 2026-10-31: adding it lifted
//! `Sunset Boulevard (musical film adaptation)` above the 1950 film, because that candidate's
//! description literally contains the hint's words. A hint drawn from what we saw is not independent
//! evidence about which candidate we meant.

use crate::knowledge::scoring::{Order, Policy};
use crate::knowledge::text::{self, content_tokens, grams};
use crate::model;

use super::WikidataCandidate;

// Every number below prices a *Wikidata answer* on the scale `fit` produces, so they live with the
// measure that produces them; the graph's `Similarity` cannot state them.

/// The winner must beat the runner-up by this much on `fit`: a near-tie is not evidence.
pub(super) const FIT_MARGIN: f32 = 0.05;
/// The hint fallback scores on its own scale, so its margin is its own number too.
pub(super) const CLAIM_MARGIN: f32 = 0.10;

/// No answer is certain: the best a score can be priced at is 0.95.
pub(super) const ANSWER_CEILING: f32 = 0.95;
/// A guess from the disambiguating words alone is worth less than an exact label.
pub(super) const HINT_CEILING: f32 = 0.85;
/// One exact label with nothing competing for it is identity, but nothing corroborates it either.
pub(super) const LONE_LABEL_CONFIDENCE: f32 = 0.75;
/// Exactly one candidate's label is our own surface, and its description corroborates it.
pub(super) const LOOKUP_CONFIDENCE: f32 = 0.85;
/// A property match: the item *is* the account.
pub(super) const HANDLE_CONFIDENCE: f32 = 0.95;

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
/// The hint is only trusted when at least half of its disambiguating words are accounted for.
pub(super) const HINT_MIN_SUPPORT: f32 = 0.50;
/// The hint fallback is gated on the blend: a name plus partial support is not enough.
pub(super) const CLAIM_BAR: f32 = 0.60;
/// A name blocks, evidence decides: the two sum to one, so neither wins alone.
pub(super) const NAME_WEIGHT: f32 = 0.40;
pub(super) const SUPPORT_WEIGHT: f32 = 0.60;

/// The authority's one policy: the description evidence that orders, and the blend that prices it.
///
/// Carries the corpus — the whole answer — because that is what rarity is measured against, and one
/// step's selection set is a fraction of it. The lone-surface shortcut and [`choose`] share one
/// instance, so a candidate has one fit per answer rather than one per path.
pub(super) struct Ranking {
    weights: Weights,
}

impl Ranking {
    /// Measured over the answer the question returned, never over the set a decision is choosing
    /// between: a corpus of one or two candidates makes the IDF floor swallow every term, and a set
    /// filtered by name-relatedness makes the measure depend on the rule it feeds.
    pub(super) fn over(candidates: &[WikidataCandidate]) -> Self {
        Self {
            weights: Weights::over(candidates),
        }
    }

    /// One candidate's fit against the answer it arrived in: `aggregate::decide` asks this before
    /// accepting a lone same-surface candidate.
    pub(super) fn fit(
        &self,
        entity: &model::knowledge_entity::Model,
        candidate: &WikidataCandidate,
    ) -> f32 {
        fit(&our_tokens(entity), candidate, &self.weights)
    }

    /// The winner's blend against the answer it arrived in: the confidence `answer::identity` stores.
    pub(super) fn blend(
        &self,
        entity: &model::knowledge_entity::Model,
        candidate: &WikidataCandidate,
    ) -> f32 {
        blend(entity, candidate, self.fit(entity, candidate))
    }
}

impl Policy<model::knowledge_entity::Model, WikidataCandidate> for Ranking {
    type Evidence = Fit;

    fn evaluate(
        &self,
        entity: &model::knowledge_entity::Model,
        candidates: &[WikidataCandidate],
    ) -> Vec<Fit> {
        let ours = our_tokens(entity);
        candidates
            .iter()
            .map(|candidate| {
                let fit = fit(&ours, candidate, &self.weights);
                Fit {
                    fit,
                    blend: blend(entity, candidate, fit),
                }
            })
            .collect()
    }

    fn qualifies(&self, evidence: &Fit) -> bool {
        evidence.fit > 0.0
    }

    fn order(&self, evidence: &Fit) -> Order {
        // `fit` orders because it is the quantity `choose` margins; the blend only separates equals,
        // so two candidates with identical evidence do not fall back on the order they arrived in.
        Order {
            primary: evidence.fit,
            tiebreak: evidence.blend,
        }
    }
}

/// What one candidate is worth against one entity: the evidence [`choose`] margins, and the blend that
/// prices it and breaks ties.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Fit {
    fit: f32,
    blend: f32,
}

/// The same name once the shared fold has run: `J.R.R. Tolkien` and `J. R. R. Tolkien` are one, and
/// so are `Pokémon` and `Pokemon`.
pub(super) fn exact_label(
    entity: &model::knowledge_entity::Model,
    candidate: &WikidataCandidate,
) -> bool {
    candidate
        .label
        .as_deref()
        .is_some_and(|label| text::comparable(label) == text::comparable(&entity.name))
}

/// The description evidence for one candidate, given the corpus the answer supplied.
fn fit(ours: &[Vec<String>], candidate: &WikidataCandidate, weights: &Weights) -> f32 {
    let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
    ours.iter()
        .map(|tokens| weights.fit(&theirs, tokens))
        .fold(0.0f32, f32::max)
}

/// The blend: name agreement plus description evidence. The identity's confidence input, not the order.
fn blend(entity: &model::knowledge_entity::Model, candidate: &WikidataCandidate, fit: f32) -> f32 {
    let name = if exact_label(entity, candidate) {
        1.0
    } else {
        text::jaccard(
            &content_tokens(candidate.label.as_deref().unwrap_or_default()),
            &content_tokens(&entity.name),
        )
    };
    NAME_WEIGHT * name + SUPPORT_WEIGHT * fit
}

/// The part of `search_query` that the label does not already say.
///
/// This subtraction is the whole point. `"James Davies anthropologist author"` scores a *photograph
/// of James Davies* on "james" and "davies" — words every candidate shares, because they are the
/// name. Only the remainder ("anthropologist", "author") carries information, and that is what the
/// fallback is allowed to weigh.
pub(super) fn hint_tokens(entity: &model::knowledge_entity::Model) -> Option<Vec<String>> {
    let raw = entity.search_query.as_deref()?.trim();
    if raw.is_empty() || text::comparable(raw) == text::comparable(&entity.name) {
        return None;
    }
    let named: std::collections::BTreeSet<String> =
        content_tokens(&entity.name).into_iter().collect();
    let hint: Vec<String> = content_tokens(raw)
        .into_iter()
        .filter(|token| !named.contains(token))
        .collect();
    (!hint.is_empty()).then_some(hint)
}

/// Every description the entity has accumulated, tokenized. A candidate only has to fit one of them:
/// taking the union instead would dilute a good match with text from unrelated captures.
pub(super) fn our_tokens(entity: &model::knowledge_entity::Model) -> Vec<Vec<String>> {
    entity.descriptions().map(content_tokens).collect()
}

/// Term rarity within the candidate set.
///
/// This is what separates a *subject* from a *location*: `"George Washington"` returns a president
/// and a "bronze statue in Portland, Oregon, United States", and both share two words with the
/// entity's description — but `united`/`states` appear in a large fraction of the candidates (every
/// American place) while `president` does not. Inverse document frequency over the candidates the
/// query itself returned, and no hand-written stoplist.
pub(super) struct Weights {
    documents: f32,
    frequency: std::collections::BTreeMap<String, usize>,
}

impl Weights {
    pub(super) fn over(candidates: &[WikidataCandidate]) -> Self {
        let mut frequency = std::collections::BTreeMap::new();
        for candidate in candidates {
            let tokens = content_tokens(candidate.description.as_deref().unwrap_or_default());
            for gram in grams(&tokens)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
            {
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
        let ours_set: std::collections::BTreeSet<&String> = ours.iter().collect();
        let theirs_set: std::collections::BTreeSet<&String> = theirs.iter().collect();
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
    use super::WikidataCandidate;
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
}
