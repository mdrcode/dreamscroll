//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the identity resolvers.
//!
//! Clusters, by the question each answers. Each is a module file named for the thing it defines,
//! with its parts beneath it:
//!
//! - [`blocker`] — which entities are worth considering: the [Blocker] trait, the query that answers
//!   from `knowledge_blocking_index`, and the index itself.
//! - [`linker`] — which candidate wins: the [Linker] trait and today's heuristic.
//! - [`graph`] — what the graph holds and every way it changes: [`KnowledgeGraph`], projection, merge
//!   and the verdicts.
//! - [`proximity`] — the derived batch of look-alike pairs, and the passes that produce and answer it.
//! - [`identity`] — resolution: the `IdentityResolver` trait, the run loop, and today's Wikidata
//!   resolver.
//! - [`eval`] — offline diagnostics: blocking recall, and one entity's walk through the resolver.
//!
//! [`verdicts`] is not a cluster: it is the store for the modification log and the standing vetoes.
//! The graph holds one and writes its merge rows through it on the fold's transaction; applying the
//! log is the graph's command (`graph/replay.rs`), exposed as a method alongside `veto` and
//! `unveto`.
//!
//! Four things stay at the root because more than one cluster needs them, and a stage that imports
//! a cluster to reach its own vocabulary has the dependency backwards:
//!
//! - [`evidence`] — [`Identity`](evidence::Identity) and the store for `knowledge_identities`.
//!   `identity` writes through it and `linking` reads it; it depends on `model`, `text` and
//!   `illumination::v2` and on nothing else here.
//! - [`text`] — the surface fold, the keys derived from it, and the words a measure compares; used
//!   by blocking, identity, evidence and projection.
//! - [`snapshot`] — copy the projection, empty it, diff two copies: an operation over the whole
//!   graph, not a way of changing it.
//! - [`helper`] — the illumination composition: which observations are linked together, and the
//!   identity annotation that follows. The graph links a collection; only this knows it is one
//!   reading.
//!
//! [Blocker]: blocker::Blocker
//! [Linker]: linker::Linker

pub use crate::knowledge2::blocker;
pub mod eval;
pub mod evidence;
pub mod graph;
pub mod helper;
pub mod identity;
pub mod linker;
pub mod proximity;
pub mod snapshot;
pub use crate::knowledge2::text;
pub mod verdicts;

pub use graph::{KnowledgeGraph, LinkReport, ReplayReport};
pub use helper::{LinkAllReport, link_all_pending, link_illumination, resolve_all_pending};
pub use proximity::AutoMergeReport;
