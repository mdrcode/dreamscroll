//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.

pub mod blocker;
pub mod graph;
pub use graph::{Knowledge, LinkStats, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub mod evidence;
pub mod heuristic;

pub mod link_decider;
pub mod merge;
pub mod normalize;
pub mod resolve;
pub mod snapshot;
pub mod stats;
