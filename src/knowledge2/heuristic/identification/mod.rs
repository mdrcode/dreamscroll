//! Heuristic identification: today's Wikidata policy.
//!
//! The entry point is [`super::identifier::HeuristicIdentifier`]; this module holds the policy it
//! runs and the Wikidata provider.

mod classes;
pub(crate) mod decide;
pub(crate) mod gather;
mod measure;
pub(crate) mod plan;
pub(crate) mod ranking;
mod report;
mod wikidata;

pub use report::{DryRunIdentity, IdentificationReport, Options, ProviderReport};
pub(crate) use wikidata::Wikidata;
