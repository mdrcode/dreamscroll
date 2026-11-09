//! One authority: the policy in `resolver`, the wire in `transport`, and its own vocabulary in
//! `kind` and `candidate`.

mod answer;
mod candidate;
mod kind;
mod policy;
mod query;
mod ranking;
mod resolver;
mod transport;
mod types;

pub use candidate::Candidate;
pub use kind::Kind;
pub use resolver::Wikidata;

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(super) const SEARCH_LIMIT: usize = 10;
