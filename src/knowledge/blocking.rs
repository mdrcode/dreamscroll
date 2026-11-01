//! Blocking: which entities are worth considering for an observation.
//!
//! Recall-oriented and allowed to over-generate. A blocker never decides anything, never touches IO,
//! and owns whatever representation of the corpus it needs — the trait deliberately takes no corpus
//! parameter, so a DB-backed implementation can load or cache in its constructor without the
//! executor learning about it.
//!
//! The one implementation is [`crate::knowledge::heuristic::NameBlocker`].

use crate::knowledge::view;

pub trait Blocker {
    /// Register an entity so it can be proposed later. Called once for every existing entity before
    /// a run, and again whenever the executor creates or extends one: an entity created mid-run must
    /// be findable by the observations that follow it.
    fn index(&mut self, entity: &view::EntityView);

    /// Candidates for one observation, by entity id. Pure: same entity set, same answer.
    fn candidates(&self, observation: &view::ObservationView) -> Vec<i32>;
}
