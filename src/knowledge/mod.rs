//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.
//!
//! Three clusters, by the question each answers. Each is a module file named for the thing it
//! defines, with its parts beneath it:
//!
//! - [`blocker`] — which entities are worth considering: the [Blocker] trait, the query that answers
//!   from `knowledge_index`, and the index itself.
//! - [`linker`] — which candidate wins, and making it happen: the [Linker] trait, today's heuristic,
//!   and the executor that writes one illumination.
//! - [`graph`] — what the graph holds and every way it changes: [`Knowledge`], projection, merge,
//!   proximity, verdicts, the claim vocabulary in `evidence`, and snapshots.
//!
//! `normalize.rs` and `scoring.rs` stay at the root because both clusters, and [`identity`], use
//! them: a surface fold and a confidence comparison are not anyone's private step.
//!
//! [Blocker]: blocker::Blocker
//! [Linker]: linker::Linker

pub mod blocker;
pub mod eval;
pub mod graph;
pub mod identity;
pub mod linker;
pub mod normalize;
pub mod scoring;

pub use graph::proximity::MergeReport;
pub use graph::verdicts::ReplayReport;
pub use graph::{Knowledge, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub use linker::LinkStats;
pub use scoring::{Rank, Scoring, Similarity, TotalCmp};
