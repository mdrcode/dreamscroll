//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule is inherited from [`crate::fetch`]: **a resolver cannot fetch, only ask for answers**. A
//! resolver owns its own strategy — one request or a series of them, one pass or several — but its
//! only route to the world is [`crate::fetch::Gateway::answers_for`], which consults the cache
//! first, paces the call and charges the budget. Adding a provider means implementing [`Resolver`]; it
//! cannot forget the cache, because there is no other door.
//!
//! What lives here is everything that knows what a knowledge entity *is*: the authority contract and
//! the run loop that drives it (`resolution`), and the authority that answers today (`wikidata`).
//!
//! A resolver returns an entity's identities, not an edge to a shared row. The engine replaces that
//! provider's complete set for every entity the run answered about.

pub mod inspect;
pub mod resolution;
pub mod wikidata;

pub use resolution::{
    DryRunIdentity, Options, ProviderReport, ResolutionEngine, ResolutionReport, Resolver,
};

/// Transport calls a plain resolve is allowed to make when the caller does not choose a budget.
/// Exists so that omitting the budget does real work instead of quietly resolving nothing.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

/// Provider ids the engine refuses from a resolver: they name our own pipeline, not an authority.
pub const RESERVED_PROVIDER_IDS: &[&str] = &[
    crate::knowledge::evidence::PLATFORM_LINK_PROVIDER_ID,
    "platform",
    "model",
    "operator",
    "user",
];
