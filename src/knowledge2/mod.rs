//! knowledge2: a graph that processes mutations, and nothing else.
//!
//! Callers — a linker, a resolver, a person, a model — decide what should happen and emit
//! [`GraphMutation`] values. [`KnowledgeGraph2::proc`] applies them in one transaction. The graph
//! itself holds no policy: `heuristic` is the default non-agentic planner and resolver, and
//! `blocker` is its recall.
//!
//! Design: `knowledge2.md`.

pub mod mutation;
pub mod blocker;
pub mod graph;
pub mod replay;
pub mod heuristic;
pub mod helper;
pub mod identity;
pub mod ids;
pub mod canonical;
pub mod text;
pub mod wikidata;

pub use mutation::{Cause, GraphMutation};
pub use blocker::Blocker;
pub use graph::{GraphSelector, KnowledgeGraph2, Refusal, ReprojectScope, Report, VerdictSelector};
pub use helper::{LinkAllReport, LinkHelper, LinkReport, link_all_pending, link_illumination};
pub use identity::{Identity, IdentityKey, Provider};
pub use ids::{EntityOriginId, ObservationId};
pub use heuristic::HeuristicPlanner;
