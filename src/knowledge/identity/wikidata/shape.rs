//! Part of the cache key: the revision and parameters of *this* question.
//!
//! Bump the version when the construction changes, and put a parameter that changes the answer in
//! the string too. Both are per kind, so a change invalidates exactly the answers it changes and
//! nothing else. Readable on purpose: a row in `fetch_cache` should say which question it answered,
//! and a hash says nothing when you are looking at a stale answer.
//!
//! One version prefix per kind, so bumping the label query does not throw away the handle answers.
//! `search` moved from `wbsearchentities` to MediaWiki full text, hence v3; `label` is v3 because
//! the query now asks for `P31` alongside the label.

use super::{Kind, LANGUAGE, SEARCH_LIMIT};

pub(super) fn of(kind: &Kind) -> String {
    match kind {
        // `@en` in the `VALUES` and the label service pinned to the same language.
        Kind::Label => format!("wikidata/v3-label|lang={LANGUAGE}"),
        // The site is English Wikipedia, and the items are read back in the same language.
        Kind::Title => format!("wikidata/v2-title|lang={LANGUAGE}"),
        // The property *is* the kind (`handle:P2002`), and this query reads no labels.
        Kind::Handle(_) => "wikidata/v2-handle".to_string(),
        // Full text, with the result count as part of the question.
        Kind::Search => format!("wikidata/v3-search|lang={LANGUAGE}|limit={SEARCH_LIMIT}"),
    }
}
