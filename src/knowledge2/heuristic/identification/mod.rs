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
mod report;
mod wikidata;

pub use report::{DryRunIdentity, IdentificationReport, Options, ProviderReport};
pub(crate) use wikidata::Wikidata;
