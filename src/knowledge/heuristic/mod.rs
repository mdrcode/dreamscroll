//! One policy and the two things built on it.
//!
//! The graph asks two questions — is this the same thing, and which of these is it — and this
//! module is the heuristic's answer to both, kept together because it is one guess:
//!
//! - [`scoring`] measures and ranks. Every number in the graph lives here.
//! - [`linker`] is the procedure that applies a ranking: attach, or create.
//! - [`name_blocker`] is recall: the candidates the ranking is asked about.
//!
//! Nothing else in `knowledge` names a heuristic type; they ask the store's [`Scoring`] instead.
//! Swap this module for another pair of verbs and the rest of the graph is unchanged.
//!
//! [`Scoring`]: crate::knowledge::scoring::Scoring

pub mod linker;
pub mod name_blocker;
pub mod scoring;

pub use linker::HeuristicLinker;
pub use name_blocker::NameBlocker;
pub use scoring::Heuristic;
