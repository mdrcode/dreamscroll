//! Conservative surface normalization.
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
pub fn key(surface: &str) -> String {
    let trimmed = surface.trim();
    let unquoted = trimmed.trim_matches(QUOTES);
    let unmarked = unquoted.trim_start_matches(['@', '#']).trim();
    let unterminated = unmarked.strip_suffix('.').unwrap_or(unmarked);
    collapse(&fold(unterminated))
}

/// The words of a surface, in order, with the same folding as [`key`].
pub fn tokens(surface: &str) -> Vec<String> {
    key(surface)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// A word-order- and article-insensitive key, for blocking only. Never an identity key.
pub fn token_key(surface: &str) -> String {
    let mut words = tokens(surface);
    if words.len() > 1 && ARTICLES.contains(&words[0].as_str()) {
        words.remove(0);
    }
    words.sort();
    words.join(" ")
}

fn fold(surface: &str) -> String {
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
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'o',
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

    #[test]
    fn token_key_ignores_order_and_articles() {
        assert_eq!(token_key("The Train Dreams"), token_key("train dreams"));
        assert_eq!(token_key("Dreams Train"), token_key("Train Dreams"));
        assert_ne!(token_key("C"), token_key("C#"));
    }
}
