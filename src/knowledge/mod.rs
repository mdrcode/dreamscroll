//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.
//!
//! Three clusters, by the question each answers. Each is a module file named for the thing it
//! defines, with its parts beneath it:
//!
//! - [`blocker`] — which entities are worth considering: the [Blocker] trait, the query that answers
//!   from `knowledge_blocking_index`, and the index itself.
//! - [`linker`] — which candidate wins, and making it happen: the [Linker] trait, today's heuristic,
//!   and the executor that writes one illumination.
//! - [`graph`] — what the graph holds and every way it changes: [`Knowledge`], projection, merge,
//!   proximity and verdicts.
//!
//! Three things stay at the root because more than one cluster needs them, and a stage that imports
//! a cluster to reach its own vocabulary has the dependency backwards:
//!
//! - [`evidence`] — [`Claim`], [`ClaimBasis`] and the two writers of `knowledge_claims`. `identity`
//!   writes through it and `linking` reads it; it depends on `model`, `normalize` and `scoring` and
//!   on nothing else here.
//! - [`normalize`] and [`scoring`] — a surface fold and a confidence comparison, used by blocking,
//!   identity, evidence and projection alike.
//! - [`snapshot`] — copy the projection, empty it, diff two copies: an operation over the whole
//!   graph, not a way of changing it.
//!
//! [Blocker]: blocker::Blocker
//! [Linker]: linker::Linker
//! [`Claim`]: evidence::Claim
//! [`ClaimBasis`]: evidence::ClaimBasis

pub mod blocker;
pub mod eval;
pub mod evidence;
pub mod graph;
pub mod identity;
pub mod linker;
pub mod normalize;
pub mod scoring;
pub mod snapshot;

pub use graph::proximity::MergeReport;
pub use graph::verdicts::ReplayReport;
pub use graph::{Knowledge, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub use linker::LinkStats;
pub use scoring::{Rank, Scoring, Similarity, TotalCmp};
