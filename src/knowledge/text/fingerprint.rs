//! The behavioural stamp on the fold and the key.
//!
//! [`norm_fingerprint`] hashes what `comparable` and `word_key` *do* to a probe corpus. Any change to
//! either policy moves it on its own, so nothing depends on someone remembering a version number —
//! and `knowledge_blocking_index` carries the value it was built with, which is how a stale index
//! becomes visible instead of silently wrong.

use std::sync::LazyLock;

use super::{comparable, word_key};

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
    "Dr. Dominic Ng",
    "Donald J. Trump",
    "J. R. R. Tolkien",
    "Malcolm X",
    "Đorđe",
    "Ādam",
    "The Train Dreams",
    "An American Werewolf",
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
/// FNV-1a rather than `DefaultHasher`, whose output is not specified to be stable across Rust
/// releases. Computed once per process.
static FINGERPRINT: LazyLock<String> = LazyLock::new(|| {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    // One fragment per generation, each labelled: adding, renaming or reordering a generation
    // moves the fingerprint even if its values happen to coincide with an older one.
    let mut feed = |label: &str, value: &str| {
        let bytes = label
            .bytes()
            .chain([0x1e])
            .chain(value.bytes())
            .chain([0x1f]);
        for byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for probe in PROBE {
        let folded = comparable(probe);
        feed("surface", &folded);
        feed("words", &word_key(&folded));
    }
    format!("norm-{hash:016x}")
});

pub(crate) fn norm_fingerprint() -> &'static str {
    FINGERPRINT.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fingerprint has to move when the rule moves. If this fails, the fold or the probe corpus
    /// changed: update the constant *and* rebuild `knowledge_blocking_index`. The constant is not
    /// load-bearing — `project` stores whatever the code computes — it exists so the change appears
    /// in the diff.
    #[test]
    fn the_normalisation_fingerprint_moves_only_deliberately() {
        assert_eq!(norm_fingerprint(), "norm-25f79cd45c56bc24");
    }
}
