//! One authority: the policy in `resolver`, the wire in `transport`, and its own vocabulary in
//! `kind` and `candidate`.

mod answer;
mod candidate;
mod kind;
pub(crate) mod parse;
pub(crate) mod plan;
mod policy;
mod query;
mod ranking;
mod resolver;
mod shape;
mod transport;
mod types;

pub(crate) use answer::AuthorityClaim;
pub use candidate::Candidate;
pub use kind::Kind;
pub use resolver::Wikidata;

/// This authority's id: the `provider` on every request, the `scheme` on every claim, and what
/// `Resolver::id` returns. One spelling, because all three are compared.
pub(crate) const ID: &str = "wikidata";

/// The language every request is pinned to: the label `VALUES`, the label service, and the shape
/// key that hashes the construction.
pub(super) const LANGUAGE: &str = "en";

/// How many candidates a text search asks for. Part of the cache shape, so changing it invalidates
/// cached answers by design.
pub(super) const SEARCH_LIMIT: usize = 10;
