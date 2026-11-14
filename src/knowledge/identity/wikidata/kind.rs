//! This authority's request kinds, and the two components of the cache key each one pins: the name
//! it is cached under and its *shape* — the revision and parameters of that question.
//!
//! The variant order *is* the plan's, and every dispatch on it is exhaustive — a typo can no longer
//! fall through to the label path. Nothing outside this provider reads it: the fetch layer only ever
//! sees [`WikidataKind`]'s `Display` and [`WikidataKind::shape`], which together with the query are
//! the key a cached answer is filed under.
//!
//! Bump a shape's version when the construction changes, and put a parameter that changes the answer
//! in the string too. Both are per kind, so a change invalidates exactly the answers it changes and
//! nothing else. Readable on purpose: a row in `fetch_cache` should say which question it answered,
//! and a hash says nothing when you are looking at a stale answer.

use std::fmt;

use super::property_query::PropertyId;
use super::{LANGUAGE, SEARCH_LIMIT};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WikidataKind {
    /// Exact label, through a SPARQL `VALUES`.
    Label,
    /// A Wikipedia article title, which is unique where a label is not.
    Title,
    /// MediaWiki full-text search: the expensive fallback.
    Search,
    /// A platform link's handle, held by one Wikidata property (`P2002` for X, and so on).
    Handle(PropertyId),
}

impl WikidataKind {
    /// The property a handle request asks about, if it is one.
    pub fn property(&self) -> Option<PropertyId> {
        match self {
            Self::Handle(property) => Some(*property),
            _ => None,
        }
    }

    /// The cache key's shape: which construction answered, and the constants it read.
    ///
    /// One version prefix per kind, so bumping the label query does not throw away the handle
    /// answers. `search` moved from `wbsearchentities` to MediaWiki full text, hence v3; `label` is
    /// v3 because the query now asks for `P31` alongside the label.
    pub(super) fn shape(&self) -> String {
        match self {
            // `@en` in the `VALUES` and the label service pinned to the same language.
            Self::Label => format!("wikidata/v3-label|lang={LANGUAGE}"),
            // The site is English Wikipedia, and the items are read back in the same language.
            Self::Title => format!("wikidata/v2-title|lang={LANGUAGE}"),
            // The property *is* the kind (`handle:P2002`), and this query reads no labels.
            Self::Handle(_) => "wikidata/v2-handle".to_string(),
            // Full text, with the result count as part of the question.
            Self::Search => format!("wikidata/v3-search|lang={LANGUAGE}|limit={SEARCH_LIMIT}"),
        }
    }
}

/// The name this kind answers to in the cache. Stored, so changing it invalidates answers — the same
/// rule [`WikidataKind::shape`] follows.
impl fmt::Display for WikidataKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Label => f.write_str("label"),
            Self::Title => f.write_str("title"),
            Self::Search => f.write_str("search"),
            Self::Handle(property) => write!(f, "handle:{}", property.as_str()),
        }
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
        let label = WikidataKind::Label.shape();
        let title = WikidataKind::Title.shape();
        let search = WikidataKind::Search.shape();
        let handle = WikidataKind::Handle(PropertyId::X_USERNAME).shape();

        assert!(search.contains("limit="), "{search}");
        for (kind, shape) in [("label", &label), ("title", &title), ("handle", &handle)] {
            assert!(!shape.contains("limit="), "{kind} sends no limit: {shape}");
        }
        assert!(
            !handle.contains("lang="),
            "a handle query reads no labels: {handle}"
        );
        assert_eq!(
            WikidataKind::Handle(PropertyId::INSTAGRAM_USERNAME).shape(),
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
