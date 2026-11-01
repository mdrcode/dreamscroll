//! Linking: the one place a decision is made.
//!
//! Everything upstream is recall (a blocker proposed candidates) or measurement (the scorer filled
//! in features). This trait turns those into a decision, and it is pure: same candidates, same
//! decision, no IO, no database.

use crate::knowledge::{score, view};

/// A candidate entity and how well it fits the observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub entity: i32,
    pub features: score::Features,
}

/// What to do with one observation. The metadata is carried so a caller can report *why* without
/// re-deriving it — a decision that cannot explain itself is the one we would have to debug.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Attach the observation to this entity.
    Attach { candidate: Candidate },
    /// Start a new entity. `best` is the closest rejected candidate, kept for diagnosis.
    Create { best: Option<Candidate> },
}

pub trait Linker {
    fn decide(&self, observation: &view::ObservationView, candidates: &[Candidate]) -> Decision;
}

/// The current policy: take the highest-scoring candidate that clears the bar, else create.
///
/// A name alone never decides — `score::worth_merging` requires corroboration — so this is a thin
/// shell over pure functions. It exists so that "what to do" has exactly one home.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScorerLinker;

impl Linker for ScorerLinker {
    fn decide(&self, _observation: &view::ObservationView, candidates: &[Candidate]) -> Decision {
        let best = candidates
            .iter()
            .max_by(|a, b| a.features.total.total_cmp(&b.features.total))
            .cloned();

        match best {
            Some(candidate) if score::worth_merging(&candidate.features) => {
                Decision::Attach { candidate }
            }
            best => Decision::Create { best },
        }
    }
}
