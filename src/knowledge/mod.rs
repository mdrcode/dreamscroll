//! knowledge: a graph that processes mutations, and nothing else.
//!
//! Callers — a linker, a resolver, a person, a model — decide what should happen and emit
//! [`GraphMutation`] values. [`KnowledgeGraph::proc`] applies them in one transaction. The graph
//! itself holds no policy: `heuristic` is the default non-agentic planner and resolver, and
//! `blocker` is its recall.
//!
//! Design: `knowledge.md`.

pub mod blocker;
pub mod canonical;
pub mod eval;
pub mod graph;
pub mod helper;
pub mod heuristic;
pub mod identity;
pub mod ids;
pub mod mutation;
pub mod proximity;
pub mod replay;
pub mod snapshot;
pub mod text;
pub mod wikidata;

pub use blocker::Blocker;
pub use graph::{
    GraphSelector, KnowledgeGraph, MergePreview, Refusal, Report, ReprojectScope, VerdictSelector,
};
pub use helper::{IdentificationOptions, IdentifyHelper, LinkAllReport, LinkHelper, LinkReport};
pub use heuristic::{HeuristicIdentifier, HeuristicLinker};
pub use identity::{Identity, IdentityKey, Provider};
pub use ids::{EntityOriginId, ObservationId};
pub use mutation::{Cause, GraphMutation};
