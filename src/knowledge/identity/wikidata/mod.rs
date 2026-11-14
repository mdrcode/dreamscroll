//! One authority: its request kinds (`kind`) and the item it offers (`candidate`), what to ask
//! (`plan`), what an answer means (`answer`), what the answers become (`gather` → `ranking` →
//! `decide`, measured by `measure`), and the wire it speaks through (`transport`).

mod answer;
mod candidate;
mod classes;
pub(crate) mod decide;
pub(crate) mod gather;
mod kind;
mod measure;
pub(crate) mod plan;
mod property_query;
pub(crate) mod ranking;
mod resolver;
mod sparql;
mod transport;

pub use candidate::WikidataCandidate;
pub use kind::WikidataKind;
pub use resolver::Wikidata;

/// This authority's id: the `provider` on every request, the `scheme` on every identity, and what
/// `IdentityResolver::id` returns. One spelling, because all three are compared.
pub(crate) const ID: &str = "wikidata";

/// The language every request is pinned to: the label `VALUES`, the label service, and the shape
/// key that hashes the construction.
pub(super) const LANGUAGE: &str = "en";

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(super) const SEARCH_LIMIT: usize = 10;
