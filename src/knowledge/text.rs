//! Text: how a string becomes the words and grams a measure compares.
//!
//! Not the fold. [`normalize`](crate::knowledge::normalize) answers *are these two surfaces the same
//! string*, and what it produces is a key that gets stored. This module answers *do these two texts
//! talk about the same thing*, which is a ranking question: the tokens are stemmed, filtered and
//! paired into bigrams, and none of it is ever written down.
//!
//! Every function is pure and works on one string at a time, with one exception worth knowing about:
//! [`near_surface`] compares two *already comparable* surfaces, so its caller owns the fold.

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
/// Hand-rolled, and deliberately crude: it is a *matching aid* over short descriptions, not
/// linguistics. A real stemmer (Snowball/Porter2) is the obvious upgrade and would change which
/// identities are reached, so it is a measured change rather than a tidy-up — see
/// `plan/wikidata-cleanup-20261112.md`.
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

/// Whether two surfaces that have been folded by [`normalize::comparable`] are the same name spelled
/// differently: `dostoyevsky`/`dostoevsky`, `jrr tolkien`/`jrrtolkien`. Measured on the two cases
/// that motivated it — a one-letter transliteration and an initials/spacing difference.
///
/// Deliberately tight. A surface that merely *contains* ours — `royal navy fleet aircraft carrier
/// fleet` for `royal navy` — is a different referent that describes ours, and its text overlap is
/// exactly what would fool a fit score.
///
/// [`normalize::comparable`]: crate::knowledge::normalize::comparable
pub(crate) fn near_surface(theirs: &str, ours: &str) -> bool {
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

/// Levenshtein distance over characters. Hand-rolled for the same reason [`stem`] is: replacing it
/// with `strsim` is a one-line change and a decision, not a refactor.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::normalize;

    /// The fold is shared with the graph, so the near-surface test inherits its diacritic and spacing
    /// work. The old bespoke key kept the accent, so `Pokémon` and `Pokemon` never met.
    #[test]
    fn near_surface_folds_before_measuring_edits() {
        assert!(near_surface(
            &normalize::comparable("Pokémon"),
            &normalize::comparable("Pokemon"),
        ));
        assert!(near_surface(
            &normalize::comparable("J. R. R. Tolkien"),
            &normalize::comparable("J.R.R. Tolkien"),
        ));
    }

    /// Tight on purpose: a title that embeds ours is another referent, however close the spelling.
    #[test]
    fn a_surface_that_contains_ours_is_not_near_it() {
        assert!(!near_surface(
            &normalize::comparable("Royal Navy fleet aircraft carrier fleet"),
            &normalize::comparable("Royal Navy"),
        ));
    }

    /// The stem exists so `painter` and `painting` meet, and must not damage words that merely end in
    /// the same letters.
    #[test]
    fn stems_meet_without_mangling() {
        assert_eq!(content_tokens("painting"), content_tokens("painter"));
        assert_ne!(content_tokens("water"), content_tokens("watch"));
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
}
