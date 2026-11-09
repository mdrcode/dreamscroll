//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.
//!
//! Three clusters, by the question each answers:
//!
//! - [`blocking`] — which entities are worth considering: the [Blocker] trait, the query that
//!   answers from `knowledge_index`, and the index itself.
//! - [`linking`] — which candidate wins, and making it happen: the [Linker] trait, today's
//!   heuristic, and the executor that writes one illumination.
//! - [`graph`] — what the graph holds and every way it changes: [`Knowledge`], projection, merge,
//!   proximity, verdicts, the claim vocabulary in `evidence`, and snapshots.
//!
//! `normalize.rs` and `scoring.rs` stay at the root because both clusters, and [`identity`], use
//! them: a surface fold and a confidence comparison are not anyone's private step.
//!
//! [Blocker]: blocking::Blocker
//! [Linker]: linking::Linker

pub mod blocking;
pub mod eval;
pub mod graph;
pub mod identity;
pub mod linking;
pub mod normalize;
pub mod scoring;

pub use graph::proximity::MergeReport;
pub use graph::verdicts::ReplayReport;
pub use graph::{Knowledge, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub use linking::LinkStats;
pub use scoring::{Rank, Scoring, Similarity, TotalCmp};
