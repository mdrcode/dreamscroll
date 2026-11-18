//! The word-order-insensitive form of a folded surface — the blocking key.
//!
//! `comparable` answers "same string"; this answers "same words, arranged differently": one leading
//! article and one leading honorific dropped, single-letter initials dropped, the rest sorted.
//!
//! Dropped from *this* key only. An article is noise for matching and still part of the name, so the
//! fold keeps it and this does not — that is why the two generations coexist rather than one
//! replacing the other.

/// Dropped from the word key only. Noise for matching, never an identity decision.
const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// Titles a capture puts in front of a name and the graph never stores: `Dr. Dominic Ng` and
/// `Dominic Ng` are one person, and the honorific broke both keys. Leading position only — a `dr`
/// inside a name is part of it — and only when something is left to match on.
const HONORIFICS: [&str; 5] = ["dr", "mr", "mrs", "ms", "prof"];

/// The word-order-insensitive form of a *comparable* surface: words sorted, one leading article and
/// one leading honorific dropped, single-letter initials dropped.
///
/// Initials are a surface artifact of how people write names — `Donald J. Trump`, `J. R. R.
/// Tolkien` — and no store names the person that way, so keeping them made the key unusable. The
/// distinctions that must survive are preserved: `C`, `C#` and `C++` are single tokens and stay as
/// they are, and `Malcolm X` folds to `malcolm`, which still matches `Malcolm`.
pub(crate) fn word_key(comparable: &str) -> String {
    // `comparable` tightens whitespace beside punctuation, so `Dr. Dominic` arrives as
    // `dr.dominic`; splitting on the period too keeps the honorific and the initial addressable.
    let mut words: Vec<&str> = comparable
        .split([' ', '.'])
        .filter(|word| !word.is_empty())
        .collect();
    if words.len() > 1 && ARTICLES.contains(&words[0]) {
        words.remove(0);
    }
    if words.len() > 1 && HONORIFICS.contains(&words[0]) {
        words.remove(0);
    }
    let is_initial =
        |word: &str| word.chars().count() == 1 && word.chars().all(char::is_alphabetic);
    if words.len() > 1 {
        // A surface made only of initials (`U.S.`) must not fold to nothing: an empty key blocks
        // nothing, which is worse than an awkward one.
        let kept: Vec<&str> = words
            .iter()
            .copied()
            .filter(|word| !is_initial(word))
            .collect();
        if !kept.is_empty() {
            words = kept;
        }
    }
    words.sort_unstable();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::text::comparable;

    fn key(surface: &str) -> String {
        word_key(&comparable(surface))
    }

    /// An honorific and initials are surface artifacts: no store names a person that way, and
    /// keeping them made the key unusable for the exact names people write.
    #[test]
    fn drops_honorifics_and_initials() {
        assert_eq!(key("Dr. Dominic Ng"), key("Dominic Ng"));
        assert_eq!(key("Donald J. Trump"), key("Donald Trump"));
        assert_eq!(key("J. R. R. Tolkien"), key("Tolkien"));
        // What the fold must preserve is still preserved.
        assert_ne!(key("C"), key("C#"));
        assert_ne!(key("C"), key("C++"));
    }

    /// The word key ignores order and a leading article; the fold does not, and must not.
    #[test]
    fn ignores_order_and_articles() {
        assert_eq!(key("The Train Dreams"), key("train dreams"));
        assert_eq!(key("Dreams Train"), key("Train Dreams"));
        assert_ne!(key("C"), key("C#"));
        assert_ne!(key("Dune"), key("Dune Part Two"));
    }
}
