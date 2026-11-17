//! Heuristic identity resolution: today's Wikidata policy.
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
mod resolver;

pub use report::{DryRunIdentity, Options, ProviderReport, ResolutionReport};
pub(crate) use resolver::Wikidata;
