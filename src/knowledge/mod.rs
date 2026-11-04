//! The knowledge graph: observations, the knowledge entities they link to, and the evidence.
//!
//! Design: `plan/knowledge-canonical-entities.md`; vocabulary: `plan/lexicon.md`.
//! `illuminate` writes observations, `link` clusters them, `resolve` asks the authorities.
//!
//! `graph.rs` is the facade — `Knowledge` — and the primitives every write shares. The three write
//! paths that have their own logic and their own tests live beside it: `link.rs`, `project.rs` and
//! `merge.rs`. They are private modules; callers reach them through `Knowledge`'s methods.
//!
//! Scoring is its own seam: `scoring.rs` states the two verbs (`Similarity`, `Rank`), `heuristic/`
//! is today's only answer to them, and the store hands one `Scoring` to the linker and to
//! `reconcile`.

pub mod blocker;
pub mod evidence;
pub mod graph;
pub mod heuristic;
pub mod link_decider;
pub mod merge;
mod reconcile;
pub mod normalize;
pub mod resolve;
pub mod scoring;
pub mod snapshot;

mod link;
mod project;

pub use graph::{Knowledge, STATUS_IDENTIFIED, STATUS_PROVISIONAL};
pub use reconcile::Report as ReconcileReport;
pub use reconcile::list as list_proximities;
pub use link::LinkStats;
pub use scoring::{by_confidence, Rank, Scoring, Similarity, Subject};
