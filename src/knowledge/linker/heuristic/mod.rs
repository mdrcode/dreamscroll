//! One policy and the two things built on it.
//!
//! The graph asks two questions — is this the same thing, and which of these is it — and this
//! module is the heuristic's answer to both, kept together because it is one guess:
//!
//! - [`scoring`] measures and ranks. Every number in the graph lives here.
//! - [`linker`] is the procedure that applies a ranking: attach, or create.
//!
//! The linker holds this policy as a member, and a pass that measures is handed one; nothing else in
//! `knowledge` names a heuristic type. Swap this module for another implementation and the rest of
//! the graph is unchanged — the seam is the type, not a trait.

pub mod linker;
pub mod scoring;

pub use linker::HeuristicLinker;
pub use scoring::Heuristic;
