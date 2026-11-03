//! Conservative surface normalization.
//!
//! One pipeline, three consumers:
//!
//! ```text
//! surface ──key──▶ folded surface ──tokens_of──▶ words ──token_key_of──▶ sorted words
//! ```
//!
//! - [`key`] is what [`crate::knowledge::graph`] dedupes projections with, and what
//!   [`crate::knowledge::evidence`] turns a handle into an identity with.
//! - [`token_key_of`] is the blocking key: word order and a leading article folded away, so it may
//!   propose candidates but must never decide identity.
//!
//! Nothing here knows what a platform is. A platform presence has its own key, on the claim that
//! answers for it: [`crate::knowledge::evidence::Claim::key`].
//!
//! Used for grouping and lookup only. The captured surface is never rewritten; `key` answers
//! "would a user call these the same string" and nothing more. Inner punctuation is preserved on
//! purpose: `C`, `C#` and `C++` are three different things, and folding them together is the
//! mistake the corpus already showed.

const QUOTES: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// Dropped from the *token* key only. They are noise for blocking, and never an identity decision.
const ARTICLES: [&str; 3] = ["the", "a", "an"];

/// The lookup key for a surface: case, diacritics, surrounding quotes, a leading `@`/`#` and a
/// trailing period are folded away; everything inside the name stays.
pub(crate) fn key(surface: &str) -> String {
    let trimmed = surface.trim();
    let unquoted = trimmed.trim_matches(QUOTES);
    let unmarked = unquoted.trim_start_matches(['@', '#']).trim();
    let unterminated = unmarked.strip_suffix('.').unwrap_or(unmarked);
    collapse(&fold_case(unterminated))
}

/// The words of a surface, in order, with the same folding as [`key`].
pub(crate) fn tokens(surface: &str) -> Vec<String> {
    tokens_of(&key(surface))
}

/// The words of an already-folded key — the pipeline's second step, for a caller that computed
/// [`key`] anyway.
pub(crate) fn tokens_of(key: &str) -> Vec<String> {
    key.split_whitespace().map(str::to_string).collect()
}

/// A word-order- and article-insensitive key, for blocking only. Never an identity key.
///
/// The caller tokenizes: every caller has the words already (an `Observation`, a `Scored`, a
/// `NameBlocker` keying a surface), and a surface-taking variant would fold and split it twice.
pub(crate) fn token_key_of(words: &[String]) -> String {
    let mut words: Vec<&str> = words.iter().map(String::as_str).collect();
    if words.len() > 1 && ARTICLES.contains(&words[0]) {
        words.remove(0);
    }
    words.sort_unstable();
    words.join(" ")
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

fn collapse(surface: &str) -> String {
    surface.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_diacritics_and_wrapping() {
        assert_eq!(key("  Pokémon "), "pokemon");
        assert_eq!(key("\"Train Dreams\""), "train dreams");
        assert_eq!(key("@bandy1803"), "bandy1803");
        assert_eq!(key("Spider-Man."), "spider-man");
    }

    #[test]
    fn keeps_inner_punctuation() {
        // The observed collision: aggregation must not merge these.
        assert_ne!(key("C"), key("C#"));
        assert_ne!(key("C#"), key("C++"));
        assert_ne!(key("U.S."), key("US"));
    }

    /// The blocking key, spelled the way a caller spells it: fold, split, sort.
    fn blocking(surface: &str) -> String {
        token_key_of(&tokens_of(&key(surface)))
    }

    #[test]
    fn token_key_ignores_order_and_articles() {
        assert_eq!(blocking("The Train Dreams"), blocking("train dreams"));
        assert_eq!(blocking("Dreams Train"), blocking("Train Dreams"));
        assert_ne!(blocking("C"), blocking("C#"));
    }
}
