//! v1 adapter over the extracted Wikidata core.
//!
//! The wire, query language, parsed candidate and class table live in `knowledge2::wikidata`.
//! The heuristic plan and ranking still live here until the resolution move; `Wikidata` implements
//! the v1 `IdentityResolver` and the engine owns the write path.

pub(crate) use crate::knowledge2::wikidata::{ID, Wire, answer, classes, property_query};
pub use crate::knowledge2::wikidata::{WikidataCandidate, WikidataKind};

pub(crate) mod decide;
pub(crate) mod gather;
mod measure;
pub(crate) mod plan;
pub(crate) mod ranking;
mod resolver;

pub use resolver::Wikidata;
