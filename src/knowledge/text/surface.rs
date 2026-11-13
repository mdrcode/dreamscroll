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
}
