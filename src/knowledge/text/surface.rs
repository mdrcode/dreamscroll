//! Is one surface a spelling of the other?
//!
//! [`near_surface`] compares two *already folded* surfaces, so its caller owns the fold. It is the
//! tolerance a measure needs when the authority spells a name differently: a transliteration
//! (`Dostoevsky`/`Dostoyevsky`) or an initials/spacing convention (`J.R.R.`/`J. R. R.`).

use strsim::levenshtein;

/// Whether two surfaces folded by [`comparable`](super::comparable) are the same name spelled
/// differently: `dostoyevsky`/`dostoevsky`, `jrr tolkien`/`jrrtolkien`. Measured on the two cases
/// that motivated it — a one-letter transliteration and an initials/spacing difference.
///
/// Deliberately tight. A surface that merely *contains* ours — `royal navy fleet aircraft carrier
/// fleet` for `royal navy` — is a different referent that describes ours, and its text overlap is
/// exactly what would fool a fit score.
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
    levenshtein(theirs, ours) <= allowance
}

/// One folded surface is a whole-word prefix of the other: `m4` in `m4 carbine`, never `roy` in
/// `royal`.
///
/// Both sides are already comparable, like [`near_surface`]'s: the caller owns the fold. The prefix
/// must be *proper* — an equal surface returns false, because "one contains the other" is what this
/// prices, and equality is a stronger claim the caller has already made. A prefix is weaker than a
/// spelling difference: the longer surface is *about* a thing the shorter one names.
pub(crate) fn word_prefix(short: &str, long: &str) -> bool {
    long.strip_prefix(short)
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with(char::is_alphanumeric))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::text::comparable;

    /// The fold is shared with the graph, so the near-surface test inherits its diacritic and spacing
    /// work. The old bespoke key kept the accent, so `Pokémon` and `Pokemon` never met.
    #[test]
    fn folds_before_measuring_edits() {
        assert!(near_surface(&comparable("Pokémon"), &comparable("Pokemon"),));
        assert!(near_surface(
            &comparable("J. R. R. Tolkien"),
            &comparable("J.R.R. Tolkien"),
        ));
    }

    /// Tight on purpose: a title that embeds ours is another referent, however close the spelling.
    #[test]
    fn a_surface_that_contains_ours_is_not_near_it() {
        assert!(!near_surface(
            &comparable("Royal Navy fleet aircraft carrier fleet"),
            &comparable("Royal Navy"),
        ));
    }

    /// The boundary is a word, not a character: `m4` is a whole-word prefix of `m4 carbine`, while
    /// `roy` only starts the word `royal` and names nothing.
    #[test]
    fn a_prefix_stops_at_a_word_boundary() {
        assert!(word_prefix("m4", "m4 carbine"));
        assert!(word_prefix("paris", "paris saint germain"));
        assert!(!word_prefix("roy", "royal navy"));
        assert!(!word_prefix("m4", "m14"));
        assert!(!word_prefix("m4", "m4"));
    }
}
