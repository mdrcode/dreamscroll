//! The comparable form of a surface.
//!
//! ```text
//! "@Bandy1803." ──comparable──▶ "bandy1803"
//! ```
//!
//! [`comparable`] is what [`crate::knowledge::graph`] dedupes projections with, and what
//! [`crate::knowledge::evidence`] turns a handle into an identity with: two surfaces are the same
//! thing exactly when their comparable forms are equal.
//!
//! The token pipeline above it — words, then a word-order-insensitive blocking key — is a guess
//! about what might be the same thing, so it lives with the heuristic.
//!
//! Nothing here knows what a platform is. A platform presence has its own key, on the claim that
//! answers for it: [`crate::knowledge::evidence::Claim::key`].
//!
//! Used for grouping and lookup only. The captured surface is never rewritten; `comparable` answers
//! "would a user call these the same string" and nothing more. Inner punctuation is preserved on
//! purpose: `C`, `C#` and `C++` are three different things, and folding them together is the
//! mistake the corpus already showed.

const QUOTES: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// The form two surfaces are compared in: case, diacritics, surrounding quotes, a leading `@`/`#`
/// and a trailing period are folded away; everything inside the name stays. Never stored, never
/// displayed.
pub(crate) fn comparable(surface: &str) -> String {
    let trimmed = surface.trim();
    let unquoted = trimmed.trim_matches(QUOTES);
    let unmarked = unquoted.trim_start_matches(['@', '#']).trim();
    let unterminated = unmarked.strip_suffix('.').unwrap_or(unmarked);
    collapse(&fold_case(unterminated))
}

fn collapse(surface: &str) -> String {
    surface.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold_case(surface: &str) -> String {
    let mut out = String::with_capacity(surface.len());
    for c in surface.chars() {
        match latin_base(c) {
            Some(base) => out.push(base),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Latin-1 diacritics only. NFKC and full case folding need `unicode-normalization`; that is a new
/// dependency and this table covers the corpus (Pokémon, Café, naïve, Şeyh…).
fn latin_base(c: char) -> Option<char> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
            'a'
        }
        'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => 'c',
        'ď' | 'Đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
        'ğ' | 'Ğ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
        'ł' | 'Ł' => 'l',
        'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => {
            'o'
        }
        'ř' | 'Ř' => 'r',
        'ś' | 'š' | 'ş' | 'Ś' | 'Š' | 'Ş' => 's',
        'ť' | 'Ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => 'z',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_diacritics_and_wrapping() {
        assert_eq!(comparable("  Pokémon "), "pokemon");
        assert_eq!(comparable("\"Train Dreams\""), "train dreams");
        assert_eq!(comparable("@bandy1803"), "bandy1803");
        assert_eq!(comparable("Spider-Man."), "spider-man");
    }

    #[test]
    fn keeps_inner_punctuation() {
        // The observed collision: aggregation must not merge these.
        assert_ne!(comparable("C"), comparable("C#"));
        assert_ne!(comparable("C#"), comparable("C++"));
        assert_ne!(comparable("U.S."), comparable("US"));
    }
}
