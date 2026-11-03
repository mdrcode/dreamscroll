//! Resolution: ask authorities about entities, through the cache, and never any other way.
//!
//! The rule this module enforces structurally: **a resolver cannot fetch, only ask**. A resolver owns
//! its own strategy — one request or a ladder of them, one pass or several — but its only route to
//! the world is `ResolutionContext::ask`, which consults the cache first, paces the call and charges
//! the budget. Adding IMDb means adding one `Resolver`; it cannot forget the cache, because there is
//! no other door.

pub mod cache;
pub mod driver;
pub mod wikidata;

pub use driver::*;
