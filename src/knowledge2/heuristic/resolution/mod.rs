//! Heuristic identity resolution: the engine and today's Wikidata policy.
//!
//! The engine is the automated provider's execution strategy: select, batch, call, replace, report.
//! An agentic provider does not need it — it needs the graph's awaiting query, the Wikidata core and
//! the graph mutations. Everything here retires when the heuristic does.

mod classes;
pub(crate) mod decide;
mod engine;
pub(crate) mod gather;
mod measure;
pub(crate) mod plan;
pub(crate) mod ranking;
mod report;
mod resolver;

pub use engine::{ResolveEngine, resolve_all_pending};
pub use report::{DryRunIdentity, Options, ProviderReport, ResolutionReport};
pub use resolver::Wikidata;

use crate::fetch::Gateway;
use crate::knowledge2::identity::Identity;
use crate::model;

/// The policy seam the heuristic engine calls.
///
/// An agentic provider does not implement this: it owns its own control flow. The trait exists so
/// the engine can hold a policy and so tests can substitute one.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    /// The authority's id: the `provider` on every request and every identity it writes.
    fn id(&self) -> &'static str;

    /// The complete set of identities this authority asserts for each entity.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Identity)>;
}

/// Transport calls a plain resolve is allowed to make when the caller does not choose a budget.
pub const DEFAULT_MAX_REQUESTS: usize = 200;
