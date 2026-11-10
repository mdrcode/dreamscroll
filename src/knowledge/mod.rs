//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.
//!
//! Clusters, by the question each answers. Each is a module file named for the thing it defines,
//! with its parts beneath it:
//!
//! - [`blocker`] — which entities are worth considering: the [Blocker] trait, the query that answers
//!   from `knowledge_blocking_index`, and the index itself.
//! - [`linker`] — which candidate wins, and making it happen: the [Linker] trait, today's heuristic,
//!   and the executor that writes one illumination.
//! - [`graph`] — what the graph holds and every way it changes: [`KnowledgeGraph`], projection, merge
//!   and the verdicts.
//! - [`proximity`] — the derived batch of look-alike pairs, and the passes that produce and answer it.
//!
//! [`verdicts`] is not a cluster: it is the modification log and the standing vetoes, with the
//! command that applies them ([`verdicts::replay`]), so it sits at the root and takes the store and
//! the connection as arguments.
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
//! - [`helper`] — the illumination composition: which observations are linked together, and the
//!   claim assertion that follows. The graph links a collection; only this knows it is one reading.
//!
//! [Blocker]: blocker::Blocker
//! [Linker]: linker::Linker
//! [`Claim`]: evidence::Claim
//! [`ClaimBasis`]: evidence::ClaimBasis

pub mod blocker;
pub mod eval;
pub mod evidence;
pub mod graph;
pub mod helper;
pub mod identity;
pub mod linker;
pub mod normalize;
pub mod proximity;
pub mod scoring;
pub mod snapshot;
pub mod verdicts;

pub use proximity::AutoMergeReport;
pub use verdicts::ReplayReport;
pub use graph::{KnowledgeGraph, LinkStats, Status};
pub use scoring::{Rank, Scoring, Similarity, TotalCmp};
