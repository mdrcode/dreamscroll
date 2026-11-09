//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule is inherited from [`crate::fetch`]: **a resolver cannot fetch, only ask for answers**. A
//! resolver owns its own strategy — one request or a ladder of them, one pass or several — but its
//! only route to the world is [`crate::fetch::Gateway::resolve_requests`], which consults the cache
//! first, paces the call and charges the budget. Adding a provider means adding one `Resolver`; it
//! cannot forget the cache, because there is no other door.
//!
//! What lives here is everything that knows what a knowledge entity *is*: the run loop and the
//! candidate query (`engine`), where a claim becomes a row (`store`), the claim's shape
//! (`contract`), and the authority that answers today (`wikidata`).

pub mod contract;
pub mod engine;
pub mod store;
pub mod wikidata;

pub use contract::Claim;
pub use engine::{DEFAULT_MAX_REQUESTS, Options, Report, ResolutionEngine, Resolver};
