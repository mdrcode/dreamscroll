//! Wikidata: the wire, the query language, the candidate and answer admission.
//!
//! Common to every policy that asks Wikidata. The cache door is `crate::fetch`; this module owns
//! the provider id, the language, the request kinds, the SPARQL, the transport, the parsed
//! candidate and the disambiguation filter. It does not rank, score or decide.

pub(crate) mod disambiguation;
mod candidate;
mod kind;
pub(crate) mod property_query;
mod sparql;
mod transport;

pub use candidate::WikidataCandidate;
pub use kind::WikidataKind;
pub(crate) use transport::Wire;

/// This authority's id: the `provider` on every request, the `scheme` on every identity, and what
/// a resolver reports. One spelling, because all three are compared.
pub(crate) const ID: &str = "wikidata";

/// The language every request is pinned to: the label `VALUES`, the label service, and the shape
/// key that hashes the construction.
pub(crate) const LANGUAGE: &str = "en";

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(crate) const SEARCH_LIMIT: usize = 10;
