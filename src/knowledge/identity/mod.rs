//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule this module enforces structurally: **a resolver cannot fetch, only ask for answers**. A
//! resolver owns its own strategy — one request or a ladder of them, one pass or several — but its
//! only route to the world is `Gateway::resolve_requests`, which consults the cache first,
//! paces the call and charges the budget. Adding IMDb means adding one `Resolver`; it cannot forget
//! the cache, because there is no other door.

pub mod cache;
pub mod gateway;
pub mod contract;
pub mod engine;
pub mod store;
pub mod wikidata;

pub use gateway::Gateway;
pub use contract::{Answers, Claim, FetchError, Raw, Request, Transport};
pub use engine::{Options, Report, ResolutionEngine, Resolver, DEFAULT_MAX_REQUESTS};

pub(crate) use contract::BATCH;
