//! One authority: its request kinds (`kind`) and the item it offers (`candidate`), what to ask
//! (`plan`), what an answer means (`interpret`, `policy`), and the wire it speaks through (`transport`).

mod answer;
mod candidate;
pub(crate) mod interpret;
mod kind;
pub(crate) mod plan;
mod policy;
mod property_query;
mod query;
mod ranking;
mod resolver;
mod shape;
mod transport;
mod types;

pub use candidate::Candidate;
pub use kind::RequestKind;
pub use resolver::Wikidata;

/// This authority's id: the `provider` on every request, the `scheme` on every identity, and what
/// `Resolver::id` returns. One spelling, because all three are compared.
pub(crate) const ID: &str = "wikidata";

/// The language every request is pinned to: the label `VALUES`, the label service, and the shape
/// key that hashes the construction.
pub(super) const LANGUAGE: &str = "en";

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(super) const SEARCH_LIMIT: usize = 10;
