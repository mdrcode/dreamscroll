//! This authority's request kinds: the plan's steps, and the name each is cached under.
//!
//! The variant order *is* the plan's, and every dispatch on it is exhaustive — a typo can no
//! longer fall through to the label path. Nothing outside this provider reads it: the fetch layer
//! only ever sees [`WikidataKind`]'s `Display`, which is the string a cached answer is keyed by.

use std::fmt;

use super::property_query::PropertyId;

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
}

/// The name this kind answers to in the cache and in `shape`. Stored, so changing it invalidates
/// answers — the same rule `shape` follows.
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
