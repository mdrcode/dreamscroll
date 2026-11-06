//! Scoring: which of a query's candidates, if any, is the entity we meant.

use super::resolver::Candidate;
use crate::knowledge::normalize;
use crate::model;

/// Case-insensitive, whitespace-insensitive surface comparison.
fn same_surface(a: &str, b: &str) -> bool {
    a.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        == b.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
}

/// Whether two surface keys are the same name spelled differently: `dostoyevsky`/`dostoevsky`,
/// `jrr tolkien`/`jrrtolkien`. Measured on the two cases that motivated the fallback: a one-letter
/// transliteration and an initials/spacing difference.
///
/// Deliberately tight. A candidate whose label merely *contains* our name — `Royal Navy fleet
/// aircraft carrier fleet` for `Royal Navy` — is a different referent that describes ours, and its
/// text overlap is precisely what would fool the fit score.
pub(super) fn near_surface(theirs: &str, ours: &str) -> bool {
    if theirs == ours {
        return true;
    }
    let (a, b) = (theirs.chars().count(), ours.chars().count());
    let longest = a.max(b);
    if longest < 6 {
        return false;
    }
    // One edit per eight characters, and never more than two: enough for `Dostoevsky`, far too
    // little for a title that embeds ours.
    let allowance = (longest / 8).clamp(1, 2);
    edit_distance(theirs, ours) <= allowance
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

// The authority's calibration. Not the graph's: every number below prices a *Wikidata answer*, on a
// scale produced by `fit` and the blend above — so it lives with the measure that produces them, and
// the graph's `Similarity`/`Rank` cannot state it.

/// How much of the candidate's description must be accounted for by ours before a claim is worth
/// making. Two shared content words usually clears it; one never does.
pub(super) const FIT_FLOOR: f32 = 0.20;
/// …and it must beat the runner-up by this much. A near-tie is not evidence.
pub(super) const FIT_MARGIN: f32 = 0.05;
/// The hint fallback is a blend, and is held to the same margin.
pub(super) const CLAIM_MARGIN: f32 = 0.10;

/// A ranked answer, mapped into `[0.5, ANSWER_CEILING]`.
const ANSWER_BASE: f32 = 0.50;
const ANSWER_SPREAD: f32 = 0.45;
pub(super) const ANSWER_CEILING: f32 = 0.95;
/// A guess from the disambiguating words alone is worth less than an exact label.
pub(super) const HINT_CEILING: f32 = 0.85;
/// One exact label with nothing competing for it is identity, but nothing corroborates it either.
pub(super) const LONE_LABEL_CONFIDENCE: f32 = 0.75;
/// A search answer accepted because exactly one candidate's label is our own surface.
pub(super) const LOOKUP_CONFIDENCE: f32 = 0.85;
/// A property match: the item *is* the account.
pub(super) const HANDLE_CONFIDENCE: f32 = 0.95;

/// What a ranked answer is worth, before the graph adds corroboration.
pub(super) fn answer_confidence(score: f32) -> f32 {
    (ANSWER_BASE + ANSWER_SPREAD * score).min(ANSWER_CEILING)
}

/// Weight of matched evidence at which a claim is half-convincing. A scale, not a threshold: two
/// common words land well below it, two rare ones well above.
const EVIDENCE_SCALE: f32 = 3.0;
/// The hint is only trusted when at least half of its disambiguating words are accounted for.
pub(super) const HINT_MIN_SUPPORT: f32 = 0.50;
/// The hint fallback is gated on the blend: a name plus partial support is not a claim.
pub(super) const CLAIM_BAR: f32 = 0.60;
/// A name blocks; evidence decides. The name is worth enough to matter and not enough to win alone.
pub(super) const NAME_WEIGHT: f32 = 0.40;
pub(super) const SUPPORT_WEIGHT: f32 = 0.60;

/// Words that carry no disambiguation: dropping them makes description overlap mean something.
const NOISE: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "his", "her", "its", "was", "were", "who",
    "which", "about", "into", "are", "but", "not", "由", "der", "die", "das", "les", "los", "las",
];

pub(super) struct Ranked<'a> {
    pub(super) candidate: &'a Candidate,
    /// The blended score, for ordering and confidence.
    pub(super) score: f32,
    /// The description evidence on its own. The decision is made on this, because the blend includes
    /// the name — which every candidate here already shares.
    pub(super) fit: f32,
    pub(super) exact_label: bool,
}

/// Score every candidate by how well it fits, best first.
///
/// The signal is the capture's description, read two ways: *containment* — how much of what Wikidata
/// says about the candidate is accounted for by our description of it, which survives our description
/// being long — and symmetric *overlap*, which separates `Dune` (1965 novel) from `Dune` (the series)
/// because the series' description is a superset of ours.
///
/// `search_query` is deliberately not part of the score. Measured 2026-10-31: adding it lifted
/// `Sunset Boulevard (musical film adaptation)` above the 1950 film, because that candidate's
/// description literally contains the hint's words. A hint drawn from what we saw is not independent
/// evidence about which candidate we meant.
pub(super) fn rank<'a>(entity: &model::knowledge_entity::Model, candidates: &'a [Candidate]) -> Vec<Ranked<'a>> {
    let ours = our_tokens(entity);
    let weights = Weights::over(candidates);

    let mut scored: Vec<Ranked<'a>> = candidates
        .iter()
        .map(|candidate| {
            let exact_label = candidate
                .label
                .as_deref()
                .is_some_and(|label| same_surface(label, &entity.name));
            let name = if exact_label {
                1.0
            } else {
                normalize::jaccard(
                    &content_tokens(candidate.label.as_deref().unwrap_or_default()),
                    &content_tokens(&entity.name),
                )
            };
            let theirs = content_tokens(candidate.description.as_deref().unwrap_or_default());
            let fit = ours
                .iter()
                .map(|tokens| weights.fit(&theirs, tokens))
                .fold(0.0f32, f32::max);
            Ranked {
                candidate,
                score: NAME_WEIGHT * name + SUPPORT_WEIGHT * fit,
                fit,
                exact_label,
            }
        })
        .collect();
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    scored
}
/// The part of `search_query` that the label does not already say.
///
/// This subtraction is the whole point. `"James Davies anthropologist author"` scores a *photograph
/// of James Davies* on "james" and "davies" — words every candidate shares, because they are the
/// name. Only the remainder ("anthropologist", "author") carries information, and that is what the
/// fallback is allowed to weigh.
pub(super) fn hint_tokens(entity: &model::knowledge_entity::Model) -> Option<Vec<String>> {
    let raw = entity.search_query.as_deref()?.trim();
    if raw.is_empty() || same_surface(raw, &entity.name) {
        return None;
    }
    let named: std::collections::BTreeSet<String> = content_tokens(&entity.name).into_iter().collect();
    let hint: Vec<String> = content_tokens(raw)
        .into_iter()
        .filter(|token| !named.contains(token))
        .collect();
    (!hint.is_empty()).then_some(hint)
}

/// A surface with punctuation, spacing and case removed: `J. R. R. Tolkien` and `J.R.R. Tolkien` are
/// the same name, and Wikidata writes the first.
pub(super) fn surface_key(surface: &str) -> String {
    surface
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Every description the entity has accumulated, tokenized. A candidate only has to fit one of them:
/// taking the union instead would dilute a good match with text from unrelated captures.
pub(super) fn our_tokens(entity: &model::knowledge_entity::Model) -> Vec<Vec<String>> {
    entity.descriptions().map(content_tokens).collect()
}

pub(super) fn content_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.chars().count() >= 3 && !NOISE.contains(&token.as_str()))
        .map(stem)
        .collect()
}

/// Light suffix stripping, so `painter`, `painting` and `paint` are the same word.
///
/// Measured on `Caravaggio`: the capture says "revolutionized Western **painting**", Wikidata says
/// "Italian **painter**", and without this they share a single token — under a two-token minimum that
/// is zero evidence, so the best possible description scored nothing.
/// Unigrams plus adjacent bigrams. `"first president"` is stronger evidence than `first` and
/// `president` appearing separately: the phrase is a claim about the same thing, whereas two loose
/// words can be coincidence — our description and a *statue's* both contain `united` and `states`.
fn grams(tokens: &[String]) -> Vec<String> {
    let mut grams = tokens.to_vec();
    grams.extend(tokens.windows(2).map(|pair| format!("{} {}", pair[0], pair[1])));
    grams
}

fn stem(token: String) -> String {
    for suffix in ["ings", "ing", "ed", "es", "s"] {
        if token.len() > suffix.len() + 2 && token.ends_with(suffix) {
            return token[..token.len() - suffix.len()].to_string();
        }
    }
    // Agent nouns only when enough of the word remains: `painter` → `paint` is the case this exists
    // for, while `water` → `wat` and `filmmaker` → `filmak` are damage.
    if token.len() > 6 && (token.ends_with("ers") || token.ends_with("er")) {
        let trimmed = if token.ends_with("ers") {
            &token[..token.len() - 3]
        } else {
            &token[..token.len() - 2]
        };
        if trimmed.len() >= 4 {
            return trimmed.to_string();
        }
    }
    token
}

/// Term rarity within the candidate set.
///
/// This is what separates a *subject* from a *location*: `"George Washington"` returns a president
/// and a "bronze statue in Portland, Oregon, United States", and both share two words with the
/// capture — but `united`/`states` appear in a large fraction of the candidates (every American
/// place) while `president` does not. Inverse document frequency, computed over the candidates the
/// query itself returned, weights them accordingly. No hand-written stoplist, no global tuning.
pub(super) struct Weights {
    documents: f32,
    frequency: std::collections::BTreeMap<String, usize>,
}

impl Weights {
    pub(super) fn over(candidates: &[Candidate]) -> Self {
        let mut frequency = std::collections::BTreeMap::new();
        for candidate in candidates {
            let tokens = content_tokens(candidate.description.as_deref().unwrap_or_default());
            for gram in grams(&tokens).into_iter().collect::<std::collections::BTreeSet<_>>() {
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
        boost * (1.0 + self.documents / (1.0 + df)).ln()
    }

    /// The weight of distinctive material this candidate accounts for, saturating.
    ///
    /// Deliberately *not* normalised by how verbose the candidate's description is. `George
    /// Washington`'s item matches two rare words (`first`, `president`); the statue in Portland
    /// matches two very common ones (`united`, `states`). Dividing by each description's own total
    /// compressed precisely that difference — the president's text carries four further rare words we
    /// do not share (`founding`, `father`, `1732`, `1799`), which lowered its score for being rich.
    pub(super) fn fit(&self, theirs: &[String], ours: &[String]) -> f32 {
        let theirs = grams(theirs);
        let ours = grams(ours);
        let ours_set: std::collections::BTreeSet<&String> = ours.iter().collect();
        let theirs_set: std::collections::BTreeSet<&String> = theirs.iter().collect();
        let matched: Vec<&&String> = theirs_set.intersection(&ours_set).collect();
        if matched.len() < 2 {
            return 0.0;
        }
        let matched_weight: f32 = matched.iter().map(|t| self.of(t)).sum();
        matched_weight / (matched_weight + EVIDENCE_SCALE)
    }
}
