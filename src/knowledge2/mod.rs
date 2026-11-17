//! knowledge2: a graph that processes mutations, and nothing else.
//!
//! Callers — a linker, a resolver, a person, a model — decide what should happen and emit
//! [`KnowledgeAction`] values. [`KnowledgeGraph2::proc`] applies them in one transaction. There is no
//! policy, no blocker and no linker inside this module.
//!
//! Design: `knowledge2.md`.

pub mod action;
pub mod blocker;
pub mod graph;
pub mod helper;
pub mod identity;
pub mod ids;
pub mod linker;
pub mod shape;
pub mod text;

pub use action::{Cause, KnowledgeAction};
pub use blocker::Blocker;
pub use graph::{KnowledgeGraph2, Refusal, ReprojectScope, Report};
pub use helper::{LinkAllReport, LinkHelper, LinkReport, link_all_pending, link_illumination};
pub use identity::{Identity, IdentityKey, Provider};
pub use ids::{EntityOriginId, ObservationId};
pub use linker::{HeuristicPlanner, IdentityHolder, Linker};
