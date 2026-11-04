//! One authority: the policy in `resolver`, the wire in `transport`.

mod resolver;
mod transport;

pub use resolver::Wikidata;

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(super) const SEARCH_LIMIT: usize = 10;
