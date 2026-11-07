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
//!
//! Two things the corpus showed that are *not* identity, and are folded away here: whitespace
//! beside punctuation (`A. W. Moore` is `A.W. Moore`) and inner double quotes (`"Fast Eddie" Felson`
//! is `Fast Eddie Felson`). Apostrophes stay — they are part of names. Measured 2026-11-06: the
//! widenings recover two duplicate entities and cost nothing (`plan/knowledge-blocking.md`).

const QUOTES: &[char] = &['"', '\'', '“', '”', '‘', '’', '«', '»'];

/// Decoration, removed wherever it appears: a quoted title is still the title.
const INNER_QUOTES: [char; 5] = ['"', '“', '”', '«', '»'];

/// Punctuation whose surrounding whitespace carries no meaning.
const TIGHT: [char; 6] = ['.', ',', ';', ':', '!', '?'];

/// The form two surfaces are compared in. Never stored, never displayed.
///
/// Case, diacritics and ligatures are folded; surrounding quotes, a leading `@`/`#` and one trailing
/// period go; inner double quotes go; whitespace collapses, and whitespace beside punctuation
/// collapses away.
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
        let tight = before.is_some_and(|c| TIGHT.contains(&c))
            || after.is_some_and(|c| TIGHT.contains(&c));
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

/// Every branch the fold has: case, wrapping quotes, a leading `@`/`#`, one trailing period,
/// whitespace, inner double quotes, spacing at punctuation, diacritics, ligatures — and the
/// distinctions the fold must *preserve*. Add a probe when you add a branch; the fingerprint can
/// only see what is in here.
const PROBE: &[&str] = &[
    "  Pokémon ",
    "\"Train Dreams\"",
    "@bandy1803",
    "#ada lovelace",
    "Spider-Man.",
    "  @#  Mixed   CASE  ",
    "A. W. Moore",
    "A.W. Moore",
    "\"Fast Eddie\" Felson",
    "'Tis Pity She's a Whore",
    "Şeyh Bedreddin",
    "Æon Flux",
    "Straße",
    "Œuvre",
    "C",
    "C#",
    "C++",
    "U.S.",
    "US",
    "",
];

/// A fingerprint of what the fold *does*: the same probe inputs, folded, hashed.
///
/// Any change to the policy moves this on its own, so nothing depends on someone remembering a
/// version number. FNV-1a rather than `DefaultHasher`, whose output is not specified to be stable
/// across Rust releases. Computed once per process.
pub(crate) fn keys_fingerprint() -> &'static str {
    static ONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for probe in PROBE {
            for byte in comparable(probe).bytes().chain(std::iter::once(0x1f)) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("keys-{hash:016x}")
    })
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
        assert_eq!(comparable("'Tis Pity She's a Whore"), "tis pity she's a whore");
        assert_eq!(comparable("O'Brien"), "o'brien");
    }

    /// The fingerprint has to move when the rule moves. If this fails, the fold changed: update the
    /// constant *and* rebuild `knowledge_index`. The constant is not load-bearing — `project` stores
    /// whatever the code computes — it exists so the change appears in the diff.
    #[test]
    fn the_key_pipeline_fingerprint_moves_only_deliberately() {
        assert_eq!(keys_fingerprint(), "keys-5ba3c5a4965a5043");
    }

    #[test]
    fn folds_to_nothing_for_a_surface_with_no_content() {
        assert_eq!(comparable("   "), "");
        assert_eq!(comparable("\"\"\"\""), "");
        assert_eq!(comparable("@"), "");
    }

}
