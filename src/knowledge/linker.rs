//! Linking: the one place a decision is made.
//!
//! Everything upstream is recall (a blocker proposed candidates). Everything downstream is effect
//! (the executor writes). This trait turns an observation and its candidates into a decision, and
//! it is pure: same inputs, same decision, no IO.
//!
//! Note what is *not* here: no features, no score components, no thresholds. Those belong to a
//! particular heuristic, and the only thing the core asks for is a decision and the number that
//! justified it. The one implementation is [`crate::knowledge::heuristic::HeuristicLinker`].

use crate::model;

/// What to do with one observation.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Attach the observation to this entity, with the score that decided it.
    Attach { entity: i32, score: f32 },
    /// Start a new entity. `best` is the closest rejected candidate, kept for diagnosis.
    Create { best: Option<(i32, f32)> },
}

pub trait Linker {
    /// Decide what an observation is, given the entities a blocker proposed. The linker receives
    /// rows rather than scores, because measuring is part of the policy, not part of the contract.
    fn decide(
        &self,
        observation: &model::user_observed_entity::Model,
        candidates: &[&model::knowledge_entity::Model],
    ) -> Decision;
}
