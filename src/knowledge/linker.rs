//! Linking: the one place a decision is made.
//!
//! The unit is the **illumination**, not the observation. An illumination is what the system
//! produces — one illumination of one image, a coherent set of things that image showed — so a decision
//! made about one observation while ignoring its siblings is a decision about a fragment.
//!
//! Everything upstream is recall (a blocker proposed candidates). Everything downstream is effect
//! (the executor writes). This trait turns one illumination into decisions, and it does no IO of
//! its own beyond the two things it is handed: the blocker, to ask what an observation might be, and
//! the evidence, to ask whether a claim is already held.
//!
//! Note what is *not* here: no features, no score components, no thresholds. Those belong to a
//! particular linker, and the only thing the core asks for is a decision per observation. The one
//! implementation is [`crate::knowledge::heuristic::HeuristicLinker`].

use crate::model;

use super::blocking::Blocker;
use super::evidence::Evidence;

/// What to do with one observation.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Attach the observation to this entity, with the score that justified it. An identifier match
    /// scores 1.0: it is decisive, not a similarity.
    Attach { entity: i32, score: f32 },
    /// Start a new entity. `best` is the closest candidate that was turned down, kept for diagnosis
    /// — which includes the entity that holds the claim when a match was refused.
    Create { best: Option<(i32, f32)> },
}

/// What a linker decided about one observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub action: Action,
    /// What the capture asserted about it, transcribed and not yet recorded. The executor records it
    /// against whichever entity `action` resolves to, and withholds it when the decision declined.
    pub claim: Option<super::evidence::Claim>,
}

/// What a linker came back with for one illumination.
#[derive(Debug, Default, PartialEq)]
pub struct LinkResult {
    /// One decision per observation, keyed by observation id — not by position, so the executor
    /// is not relying on the order it happened to pass.
    pub decisions: Vec<(i32, Decision)>,
    /// How often an observation's best candidate was already taken by a sibling from the same
    /// illumination. The veto firing is the only thing the batch knows that the loop could not.
    pub vetoed: u32,
    /// Attachments a claim decided rather than a score. The distinction is the interesting one:
    /// these are the matches that override similarity.
    pub by_claim: u32,
}

#[async_trait::async_trait]
pub trait Linker: Send + Sync {
    /// Decide what each observation from one illumination is.
    ///
    /// `blocker` is recall and nothing else: it may return nothing, it over-generates by design, and
    /// its order means nothing. `evidence` answers whether a claim is already held. Measuring — and
    /// concluding that nothing fits — is this side's job.
    async fn link_illuminated_observations(
        &self,
        blocker: &dyn Blocker,
        evidence: &Evidence,
        illumination: &model::illumination::Model,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<LinkResult>;
}
