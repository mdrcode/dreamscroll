//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule is inherited from [`crate::fetch`]: **a resolver cannot fetch, only ask for answers**. A
//! resolver owns its own strategy — one request or a ladder of them, one pass or several — but its
//! only route to the world is [`crate::fetch::Gateway::resolve_requests`], which consults the cache
//! first, paces the call and charges the budget. Adding a provider means implementing [`Resolver`]; it
//! cannot forget the cache, because there is no other door.
//!
//! What lives here is everything that knows what a knowledge entity *is*: the authority contract and
//! the run loop that drives it (`resolution`), and the authority that answers today (`wikidata`).

pub mod resolution;
pub mod wikidata;

pub use resolution::{Options, ProviderReport, ResolutionEngine, ResolutionReport, Resolver};

/// Transport calls a plain resolve is allowed to make when the caller does not choose a budget.
/// Exists so that omitting the budget does real work instead of quietly resolving nothing.
pub const DEFAULT_MAX_REQUESTS: usize = 200;

/// Claims one resolver may assert for one entity in a run. An agentic authority must not inflate a
/// confidence by returning a hundred URLs.
const MAX_CLAIMS_PER_ENTITY: usize = 8;

/// Provider ids the engine refuses from a resolver: they name our own pipeline, not an authority.
///
/// A resolver that took one of these would make `awaiting_authority` treat capture rows or model
/// guesses as its own claims, and would make `stats` misattribute them. The check is at run time so
/// adding a resolver cannot silently steal a reserved id.
pub const RESERVED_PROVIDERS: &[&str] = &["illuminator", "platform", "model", "operator", "user"];
