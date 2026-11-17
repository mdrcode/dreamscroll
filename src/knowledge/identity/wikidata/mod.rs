//! v1 adapter over the extracted Wikidata core and the heuristic resolution.
//!
//! The core lives in `knowledge2::wikidata`; the ranking policy in
//! `knowledge2::heuristic::resolution`. `Wikidata` implements the v1 `IdentityResolver` and converts
//! the v2 identity into the v1 shape until the v2 action producer replaces the engine.

pub(crate) use crate::knowledge2::heuristic::resolution::{decide, gather, plan, ranking};
pub use crate::knowledge2::wikidata::{WikidataCandidate, WikidataKind};

mod resolver;

pub use resolver::Wikidata;
