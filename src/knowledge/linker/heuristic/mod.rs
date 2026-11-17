//! v1 adapter: [`HeuristicLinker`] applies the policy from `knowledge2::heuristic`.
//!
//! The policy and its measures live in `knowledge2::heuristic`; this module keeps the v1 procedure
//! until the v1 graph write path is deleted.

pub mod linker;

pub use crate::knowledge2::heuristic::Heuristic;
pub use linker::HeuristicLinker;
