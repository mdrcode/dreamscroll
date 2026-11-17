//! The graph's text contract: the fold and the keys derived from it.
//!
//! [`key`] is all the core uses: the blocker index stores it, the SQL blocker probes with it, and
//! projection dedupes a member's surfaces with it. The measures a planner or resolver ranks with
//! live in [`crate::knowledge2::heuristic::text`]; the graph never imports them.
pub mod key;
