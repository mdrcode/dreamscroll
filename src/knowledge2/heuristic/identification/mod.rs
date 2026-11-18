//! Heuristic identification: today's Wikidata policy.
//!
//! [`super::identifier::HeuristicIdentifier`] is the provider; the flow that selects, writes and
//! reports is `knowledge2::IdentifyHelper`.

mod classes;
pub(crate) mod decide;
pub(crate) mod gather;
mod measure;
pub(crate) mod plan;
pub(crate) mod ranking;
mod wikidata;

pub(crate) use wikidata::Wikidata;
