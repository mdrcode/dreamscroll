//! Heuristic identity resolution: ask Wikidata, gather candidates, rank, decide.
//!
//! The Wikidata core is `knowledge2::wikidata` — the same wire, cache key and parsed candidate an
//! agentic flow uses. This module is the policy: the query plan, the candidate set, the measure, the
//! ranking and the decision. It produces `knowledge2::Identity`; the graph does not depend on it.

pub(crate) mod decide;
pub(crate) mod gather;
mod measure;
pub(crate) mod plan;
pub(crate) mod ranking;
mod resolver;

pub use resolver::Wikidata;
