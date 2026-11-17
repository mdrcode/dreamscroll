//! Heuristic identity resolution: today's Wikidata policy.
//!
//! The entry point is [`super::resolver::HeuristicResolver`]; this module holds the policy it runs
//! and the internal provider seam the tests substitute.

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

use crate::fetch::Gateway;
use crate::knowledge2::identity::Identity;
use crate::model;

/// The internal policy seam the resolver holds.
///
/// An agentic provider does not implement this: it owns its own control flow. The trait exists so
/// tests can substitute a provider without a transport.
#[async_trait::async_trait]
pub(crate) trait Resolver: Send + Sync {
    /// The authority's id: the `provider` on every request and every identity it writes.
    fn id(&self) -> &'static str;

    /// The complete set of identities this authority asserts for each entity.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)>;
}
