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
pub(crate) fn content_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.chars().count() >= 3 && !NOISE.contains(&token.as_str()))
        .map(stem)
        .collect()
}

/// Light suffix stripping, so `painter`, `painting` and `paint` are the same word.
///
/// Measured on `Caravaggio`: the entity's description says "revolutionized Western **painting**",
/// Wikidata says "Italian **painter**", and without this they share a single token — under a
/// two-token minimum that is zero evidence, so the best possible description scored nothing.
///
/// Hand-rolled, and deliberately crude. A real stemmer (Snowball/Porter2) is the obvious upgrade and
/// would change which identities are reached, so it is a measured change rather than a tidy-up — see
/// the rust-stemmers experiment in `plan/wikidata-cleanup-20261112.md`.
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
