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

use super::{LANGUAGE, RequestKind, SEARCH_LIMIT};

pub(super) fn of(kind: &RequestKind) -> String {
    match kind {
        // `@en` in the `VALUES` and the label service pinned to the same language.
        RequestKind::Label => format!("wikidata/v3-label|lang={LANGUAGE}"),
        // The site is English Wikipedia, and the items are read back in the same language.
        RequestKind::Title => format!("wikidata/v2-title|lang={LANGUAGE}"),
        // The property *is* the kind (`handle:P2002`), and this query reads no labels.
        RequestKind::Handle(_) => "wikidata/v2-handle".to_string(),
        // Full text, with the result count as part of the question.
        RequestKind::Search => format!("wikidata/v3-search|lang={LANGUAGE}|limit={SEARCH_LIMIT}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A shape belongs to one kind: its own version prefix, carrying only the constants that kind's
    /// construction reads. Sharing a prefix made a change to one invalidate another, and appending a
    /// limit no kind sent made changing `SEARCH_LIMIT` throw away every label, title and handle
    /// answer for nothing — a cache miss that looks like a rate-limit problem.
    #[test]
    fn each_kind_owns_its_shape() {
        let label = of(&RequestKind::Label);
        let title = of(&RequestKind::Title);
        let search = of(&RequestKind::Search);
        let handle = of(&RequestKind::Handle("P2002"));

        assert!(search.contains("limit="), "{search}");
        for (kind, shape) in [("label", &label), ("title", &title), ("handle", &handle)] {
            assert!(!shape.contains("limit="), "{kind} sends no limit: {shape}");
        }
        assert!(
            !handle.contains("lang="),
            "a handle query reads no labels: {handle}"
        );
        assert_eq!(
            of(&RequestKind::Handle("P2003")),
            handle,
            "another property is another kind, not another shape"
        );

        let prefixes: BTreeSet<&str> = [&label, &title, &search, &handle]
            .iter()
            .filter_map(|shape| shape.split('|').next())
            .collect();
        assert_eq!(
            prefixes.len(),
            4,
            "each kind has to be bumpable on its own: {prefixes:?}"
        );
    }
}
