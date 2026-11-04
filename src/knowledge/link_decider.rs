//! Linking: the one place a decision is made.
//!
//! The unit is the **illumination**, not the observation. An illumination is what the system
//! produces — one illumination of one image, a coherent set of things that image showed — so a decision
//! made about one observation while ignoring its siblings is a decision about a fragment.
//!
//! Everything upstream is recall (a blocker proposed candidates). Everything downstream is effect
//! (the executor writes). This trait turns one illumination into decisions, and it does no IO of
//! its own beyond the two things it is handed: the blocker, to ask what an observation might be, and
//! the evidence, to ask what is already held and to name the identity a capture asserted.
//!
//! Note what is *not* here: no features, no score components, no thresholds. Those belong to a
//! particular linker, and the only thing the core asks for is a decision per observation. The one
//! implementation is [`crate::knowledge::heuristic::HeuristicLinker`].

use crate::model;

use super::blocker::Blocker;
use super::evidence::Evidence;

/// What to do with one observation.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Attach the observation to this entity.
    Attach(i32),
    /// Start a new entity.
    Create,
}

/// What the linker decided about one observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The observation itself, not its id: the subject of the decision, so the caller never has to
    /// look it up — and cannot be handed one it was never given.
    pub observation: model::user_observed_entity::Model,
    pub action: Action,
    /// The identity this capture asserted for the observation, already a row — the executor imported
    /// them all before asking. The executor records it against whichever entity `action` names, so an
    /// `Attach` can carry one too: attaching by name *and* asserting a handle is how an entity known
    /// only by its name acquires the account that speaks for it.
    pub identity: Option<i32>,
}

#[async_trait::async_trait]
pub trait LinkDecider: Send + Sync {
    /// Decide what each observation from one illumination is.
    ///
    /// `blocker` is recall and nothing else: it may return nothing, it over-generates by design, and
    /// its order means nothing. `evidence` answers what is already held, and names the identity a
    /// capture asserted — the executor imported those before calling. Measuring — and concluding that
    /// nothing fits — is this side's job.
    ///
    /// The illumination arrives whole, with its observations loaded: a reading is decided as one, and
    /// every observation in a call deliberately comes from one `illumination_id`, so an implementation
    /// that needs the set has it by the arguments rather than by the caller's good behaviour. An
    /// implementation that wants more context — the illumination's summary and details — can ask for
    /// it; nothing needs it yet.
    async fn decide_link(
        &self,
        blocker: &dyn Blocker,
        evidence: &Evidence,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<Vec<Decision>>;
}
