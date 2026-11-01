//! Resolution: ask providers about entities, through the cache, and never any other way.
//!
//! The rule this module enforces structurally: **only the executor calls an engine's `fetch`**. A
//! provider may say what it would ask (`plan`) and interpret an answer (`parse`), but it cannot
//! reach the network on its own — every request goes through the cache first. Adding IMDb means
//! adding one `Resolver`; it cannot forget the cache, because there is no other door.

pub mod driver;
pub mod wikidata;

pub use driver::*;
