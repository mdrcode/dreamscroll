//! The one door to a third-party provider: cache first, paced, budgeted, and nothing else.
//!
//! The rule this module enforces structurally: **a provider cannot fetch, only ask for answers**. A
//! provider owns its own strategy — one request or a ladder of them, one pass or several — but its
//! only route to the world is [`Gateway::resolve_requests`], which consults the cache first, paces
//! the call and charges the budget. Adding a provider cannot forget the cache, because there is no
//! other door.
//!
//! Nothing here names a provider, an entity, or a knowledge graph. A provider brings its own request
//! kinds ([`Transport::Kind`]) and its own payload ([`Transport::Payload`]); this module stores,
//! groups and returns them without reading either. That is what keeps the rule above cheap to state:
//! there is no vocabulary here to disagree with.

pub mod allowance;
pub mod cache;
pub mod contract;
pub mod gateway;
pub(crate) mod stats;

pub use allowance::{Allowance, Refusal};
pub use cache::FetchCache;
pub use contract::{Answer, Answers, FetchError, Outcome, Request, Transport};
pub use gateway::Gateway;
pub use stats::{Stats, Tally};

pub(crate) use contract::BATCH;
