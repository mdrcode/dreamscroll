//! The words a measure compares: tokenized, filtered, stemmed, paired into grams.
//!
//! Nothing here is stored and nothing here is an identity decision — these are the inputs a score is
//! built from, so they are allowed to be rough in a way the fold is not. The stem in particular is a
//! matching aid, not linguistics.

/// Words that carry no disambiguation: an overlap made of these says nothing about which referent was
/// meant. Includes a few function words from the languages captures arrive in.
const NOISE: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "his", "her", "its", "was", "were", "who",
    "which", "about", "into", "are", "but", "not", "由", "der", "die", "das", "les", "los", "las",
];

/// The words of a text that could carry signal: alphanumeric runs, lowercased, at least three
/// characters, noise dropped, stemmed.
/// Whether a word carries no disambiguation — the linker and the resolver must agree on this, or the
/// same two names can block one way and fit another.
fn is_noise(token: &str) -> bool {
    NOISE.contains(&token)
}

pub(crate) fn content_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        // Two characters is normally too short to name anything (`of`, `in`), but a model number is
        // exactly two and exactly distinctive: `M4` against `M5`, `5G` against `4G`. A digit is what
        // separates the two cases, and without this a short label never reaches the measure at all.
        .filter(|token| {
            length(token) >= 3
                || (length(token) == 2 && token.contains(|c: char| c.is_ascii_digit()))
        })
        .filter(|token| !is_noise(token))
        .map(stem)
        .collect()
}

fn length(token: &str) -> usize {
    token.chars().count()
}

/// Light suffix stripping, so `painter`, `painting` and `paint` are the same word.
///
/// Measured on `Caravaggio`: the entity's description says "revolutionized Western **painting**",
/// Wikidata says "Italian **painter**", and without this they share a single token — under a
/// two-token minimum that is zero evidence, so the best possible description scored nothing.
///
/// Hand-rolled, deliberately crude, and kept: a real stemmer (Snowball/Porter2) would change which
/// identities are reached, and adopting one was declined 2026-11-13.
///
/// Known disagreement, left in place on purpose: `carbines` stems to `carbin` while `carbine` does
/// not stem. Trimming the trailing `e` fixes it and was measured — 23 decisions moved, and merging
/// `force`/`forces` let a *painting* ("Royal Air Force Museum") outrank the aircraft whose
/// description is thin. Accepted 2026-11-13: the pair stays split rather than cost a referent
/// (`plan/wikidata-cleanup-20261112.md`, "Deliberately not changed").
fn stem(token: String) -> String {
    for suffix in ["ings", "ing", "ed", "es", "s"] {
        if token.len() > suffix.len() + 2 && token.ends_with(suffix) {
            return token[..token.len() - suffix.len()].to_string();
        }
    }
    // Agent nouns only when enough of the word remains: `painter` → `paint` is the case this exists
    // for, and the guard stops a short word losing its only syllable (`water` → `wat`). It cannot
    // stop every bad stem — `filmmaker` still becomes `filmmak`, pinned below — which is one reason
    // a real stemmer is on the table.
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
    // `es` stripped from `carbines` gives `carbin`, while `carbine` strips nothing — so a label and
    // a description said to agree never met. Trimming the `e` on both sides is what makes the stem
    // a stem rather than a suffix rule.
    token
}

/// Unigrams plus adjacent bigrams. `"first president"` is stronger evidence than `first` and
/// `president` appearing separately: the phrase is a claim about the same thing, whereas two loose
/// words can be coincidence — our description and a *statue's* both contain `united` and `states`.
pub(crate) fn grams(tokens: &[String]) -> Vec<String> {
    let mut grams = tokens.to_vec();
    grams.extend(
        tokens
            .windows(2)
            .map(|pair| format!("{} {}", pair[0], pair[1])),
    );
    grams
}

/// Set overlap: |a ∩ b| / |a ∪ b|. 0.0 when either side is empty.
pub fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a: std::collections::BTreeSet<&String> = a.iter().collect();
    let b: std::collections::BTreeSet<&String> = b.iter().collect();
    a.intersection(&b).count() as f32 / a.union(&b).count() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two characters is too short to name anything — unless it carries a digit, which is where model
    /// numbers live: `M4` is invisible to the measure without this, and `M4` against `M5` is exactly
    /// the distinction a description is asked to make.
    #[test]
    fn a_two_character_model_number_is_a_word() {
        assert_eq!(content_tokens("M4 carbine"), ["m4", "carbine"]);
        assert_eq!(content_tokens("5G network"), ["5g", "network"]);
        assert!(content_tokens("in of").is_empty(), "no digit, no word");
    }

    use super::*;

    /// The stem exists so `painter` and `painting` meet, and must not damage words that merely end in
    /// the same letters.
    #[test]
    fn stems_meet_without_mangling() {
        assert_eq!(content_tokens("painting"), content_tokens("painter"));
        assert_ne!(content_tokens("water"), content_tokens("watch"));
        // Not a claim that this is right: it is the known damage a suffix rule does, recorded so a
        // replacement stemmer's effect here is visible.
        assert_eq!(content_tokens("filmmaker"), vec!["filmmak"]);
        assert_eq!(content_tokens("the and for"), Vec::<String>::new());
    }

    /// A bigram is evidence a unigram is not: two words that only co-occur in a phrase.
    #[test]
    fn grams_add_the_adjacent_pair() {
        let tokens = content_tokens("first president of the united states");
        let grams = grams(&tokens);
        assert!(grams.contains(&"first president".to_string()), "{grams:?}");
        assert_eq!(grams.len(), tokens.len() * 2 - 1, "{grams:?}");
    }

    #[test]
    fn overlap_is_empty_when_either_side_is() {
        assert_eq!(jaccard(&[], &["a".to_string()]), 0.0);
        assert_eq!(jaccard(&["a".to_string()], &[]), 0.0);
        assert_eq!(
            jaccard(&["a".to_string(), "b".to_string()], &["a".to_string()]),
            0.5
        );
    }
}
