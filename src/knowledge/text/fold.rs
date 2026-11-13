//! The comparable form of a surface — the identity key.
//!
//! ```text
//! "@Bandy1803." ──comparable──▶ "bandy1803"
//! ```
//!
//! Case, diacritics and ligatures are folded; surrounding quotes, a leading `@`/`#` and one trailing
//! period go; inner double quotes go; whitespace collapses, and whitespace beside punctuation
//! collapses away.
//!
//! Two things the corpus showed are *not* identity, and are folded away here: whitespace beside
//! punctuation (`A. W. Moore` is `A.W. Moore`) and inner double quotes (`"Fast Eddie" Felson` is
//! `Fast Eddie Felson`). Apostrophes stay — they are part of names. Measured 2026-11-06: the
//! widenings recover two duplicate entities and cost nothing.
//!
//! Inner punctuation is otherwise preserved: `C`, `C#` and `C++` are three different things, and
//! folding them together is the mistake the corpus already showed.

const QUOTES: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// Decoration, removed wherever it appears: a quoted title is still the title.
const INNER_QUOTES: [char; 5] = ['"', '“', '”', '«', '»'];

/// Punctuation whose surrounding whitespace carries no meaning.
const TIGHT: [char; 6] = ['.', ',', ';', ':', '!', '?'];

/// The form two surfaces are compared in. Never stored, never displayed.
pub(crate) fn comparable(surface: &str) -> String {
    let trimmed = surface.trim();
    let unquoted = trimmed.trim_matches(QUOTES);
    let unmarked = unquoted.trim_start_matches(['@', '#']).trim();
    let unterminated = unmarked.strip_suffix('.').unwrap_or(unmarked);
    let folded = collapse(&fold_case(unterminated));
    expand_ligatures(&tighten(&remove_inner_quotes(&folded)))
}

fn remove_inner_quotes(surface: &str) -> String {
    surface
        .chars()
        .filter(|character| !INNER_QUOTES.contains(character))
        .collect()
}

/// Drop whitespace runs next to [`TIGHT`]; every other run becomes one space.
fn tighten(surface: &str) -> String {
    let characters: Vec<char> = surface.chars().collect();
    let mut out = String::with_capacity(surface.len());
    let mut index = 0;
    while index < characters.len() {
        if !characters[index].is_whitespace() {
            out.push(characters[index]);
            index += 1;
            continue;
        }
        while index < characters.len() && characters[index].is_whitespace() {
            index += 1;
        }
        let before = out.chars().last();
        let after = characters.get(index).copied();
        let tight =
            before.is_some_and(|c| TIGHT.contains(&c)) || after.is_some_and(|c| TIGHT.contains(&c));
        if !tight && !out.is_empty() && index < characters.len() {
            out.push(' ');
        }
    }
    out
}

fn expand_ligatures(surface: &str) -> String {
    let mut out = String::with_capacity(surface.len());
    for character in surface.chars() {
        match character {
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'ß' => out.push_str("ss"),
            _ => out.push(character),
        }
    }
    out
}

fn collapse(surface: &str) -> String {
    surface.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold_case(surface: &str) -> String {
    let mut out = String::with_capacity(surface.len());
    for c in surface.chars() {
        // Lowercase first, then map: one arm per letter covers both cases, and an uppercase whose
        // lowercase is in the table cannot slip through accented.
        for lowercase in c.to_lowercase() {
            match latin_base(lowercase) {
                Some(base) => out.push(base),
                None => out.push(lowercase),
            }
        }
    }
    out
}

/// The unaccented base of a Latin-1 diacritic, lowercase only.
///
/// `fold_case` lowercases first, so one arm per letter covers both cases: `Đ` arrives here as `đ`.
/// NFKC and full case folding need `unicode-normalization`; that is a dependency this project has
/// declined for now (`plan/wikidata-cleanup-20261112.md`), and this table covers the corpus
/// (Pokémon, Café, naïve, Şeyh…).
fn latin_base(c: char) -> Option<char> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => 'i',
        'ł' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ř' => 'r',
        'ś' | 'š' | 'ş' => 's',
        'ť' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' | 'ź' | 'ż' => 'z',
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

    /// Case folds before the table, so an uppercase whose lowercase is a mapped diacritic cannot
    /// slip through accented. `Đorđe` used to fold to `dorđe` while `Dorde` folded to `dorde`.
    #[test]
    fn folds_uppercase_and_lowercase_diacritics_alike() {
        assert_eq!(comparable("Đorđe"), "dorde");
        assert_eq!(comparable("Đorđe"), comparable("Dorde"));
        assert_eq!(comparable("Ādam"), comparable("Adam"));
        assert_eq!(comparable("ŽŪŁĆ"), comparable("ZULC"));
    }

    #[test]
    fn keeps_inner_punctuation() {
        // The observed collision: aggregation must not merge these.
        assert_ne!(comparable("C"), comparable("C#"));
        assert_ne!(comparable("C#"), comparable("C++"));
        assert_ne!(comparable("U.S."), comparable("US"));
        assert_eq!(comparable("C++"), "c++");
    }

    /// The two duplicate pairs the corpus showed, and the ligatures that would be next.
    #[test]
    fn folds_what_the_corpus_showed_is_not_identity() {
        assert_eq!(comparable("A. W. Moore"), comparable("A.W. Moore"));
        assert_eq!(comparable("A. W. Moore"), "a.w.moore");
        assert_eq!(
            comparable("\"Fast Eddie\" Felson"),
            comparable("Fast Eddie Felson")
        );
        assert_eq!(comparable("Æon Flux"), comparable("Aeon Flux"));
        assert_eq!(comparable("Straße"), "strasse");
    }

    /// Apostrophes are part of names; only double quotes are decoration.
    #[test]
    fn keeps_apostrophes() {
        assert_eq!(
            comparable("'Tis Pity She's a Whore"),
            "tis pity she's a whore"
        );
        assert_eq!(comparable("O'Brien"), "o'brien");
    }

    #[test]
    fn folds_to_nothing_for_a_surface_with_no_content() {
        assert_eq!(comparable("   "), "");
        assert_eq!(comparable("\"\"\"\""), "");
        assert_eq!(comparable("@"), "");
    }
}
