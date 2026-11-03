//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.

pub mod blocker;
// `knowledge::knowledge` reads as a stutter, and it is deliberate: the type is the graph, and the
// module is where the graph's writes live. Callers reach it through the re-export below.
#[allow(clippy::module_inception)]
pub mod knowledge;
pub use knowledge::{Knowledge, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub mod evidence;
pub mod heuristic;
pub mod link;
pub mod link_decider;
pub mod merge;
pub mod normalize;
pub mod resolve;
pub mod snapshot;
pub mod stats;
