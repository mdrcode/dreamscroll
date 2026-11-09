//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule is inherited from [`crate::fetch`]: **a resolver cannot fetch, only ask for answers**. A
//! resolver owns its own strategy — one request or a ladder of them, one pass or several — but its
//! only route to the world is [`crate::fetch::Gateway::resolve_requests`], which consults the cache
//! first, paces the call and charges the budget. Adding a provider means implementing [`Resolver`]; it
//! cannot forget the cache, because there is no other door.
//!
//! What lives here is everything that knows what a knowledge entity *is*: the contract a provider
//! implements (this file), the run loop that drives it (`engine`), and the authority that answers
//! today (`wikidata`).

pub mod engine;
pub mod wikidata;

pub use engine::{DEFAULT_MAX_REQUESTS, Options, Report, ResolutionEngine};

use crate::fetch::Gateway;
use crate::knowledge::evidence::Claim;
use crate::model;

/// One authority, and the strategy for asking it.
///
/// `resolve` takes a *batch* of entities rather than one: batching across entities is the difference
/// between a hundred transport calls for a corpus walk and five thousand, and only the resolver knows
/// how to group its own requests.
///
/// Request, answer and transport shapes are deliberately *not* here — they belong to [`crate::fetch`],
/// which knows nothing about knowledge entities. This trait knows nothing about the network, the
/// cache or the budget.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    fn id(&self) -> &'static str;

    /// Claims about the entities it could resolve, each tagged with the entity it belongs to, in
    /// priority order: the engine takes the first claim per entity that the graph accepts.
    async fn resolve(
        &self,
        entities: &[model::knowledge_entity::Model],
        gateway: &Gateway<'_>,
    ) -> Vec<(i32, Claim)>;
}
