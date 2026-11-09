//! The linker: the one place a decision is made.
//!
//! The unit is the **illumination**, not the observation. An illumination is what the system
//! produces — one illumination of one image, a coherent set of things that image showed — so a decision
//! made about one observation while ignoring its siblings is a decision about a fragment.
//!
//! Everything upstream is recall (a blocker proposed candidates). Everything downstream is effect
//! (the executor writes). This trait turns one illumination into decisions, and it does no IO of
//! its own beyond what it is handed: the blocker, to ask what an observation might be; the evidence,
//! to ask what is already held and to name the identity a capture asserted; and the store's
//! [`Scoring`], to measure.
//!
//! Note what is *not* here: no features, no weights, no thresholds. The policy is a parameter, so a
//! linker is a *procedure* — and the store's policy is the one every other module also uses. The one
//! implementation is [`heuristic::HeuristicLinker`].

pub mod heuristic;
pub(crate) mod illumination;

pub use illumination::LinkStats;

use crate::model;

use super::blocker::Blocker;
use super::evidence::Evidence;
use super::scoring::Scoring;

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
pub trait Linker: Send + Sync {
    /// Decide what each observation from one illumination is.
    ///
    /// `blocker` is recall and nothing else: it may return nothing, it over-generates by design, and
    /// its order means nothing. `evidence` answers what is already held, and names the identity a
    /// capture asserted — the executor imported those before calling. `scoring` is the store's
    /// policy: measure with it, and do not carry a second one.
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
        scoring: &Scoring,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<Vec<Decision>>;
}
