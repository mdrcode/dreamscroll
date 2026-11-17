//! The v2 resolve producer: select entities, ask a policy, emit graph actions.
//!
//! The policy is a [`Resolver`] — today `heuristic::resolution`, later an agentic flow. This module
//! is the harness: it gives the resolver the cache-backed gateway, turns the returned
//! `knowledge2::Identity` values into `AttestIdentity`/`RetractIdentity` actions, and reports the
//! run. It has no ranking and knows no authority.

mod engine;
mod report;
mod selection;

pub use engine::{ResolveEngine, resolve_all_pending};
pub use report::{DryRunIdentity, Options, ProviderReport, ResolutionReport};
pub(crate) use selection::IdentitySelection;

use crate::fetch::Gateway;
use crate::knowledge2::identity::Identity;
use crate::model;

/// One authority and its strategy.
///
/// The resolver returns a provider's complete set for each entity it answers about, not one verdict.
/// An entity it says nothing about is an abstention: its prior annotations stay.
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
/// Exists so that omitting the budget does real work instead of quietly resolving nothing.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

/// Provider ids a resolver may not use: they name our own pipeline, not an authority.
pub const RESERVED_PROVIDER_IDS: &[&str] = &["illuminator", "platform", "model", "operator", "user"];
