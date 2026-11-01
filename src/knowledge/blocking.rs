//! Blocking: which entities are worth scoring for an observation.
//!
//! Recall-oriented and allowed to over-generate. A blocker never decides anything and never touches
//! IO — it is handed entities to remember and an observation to answer about. One implementation
//! exists (names and account handles); the trait is here so a second one can be added without the
//! executor knowing.

use std::collections::{BTreeSet, HashMap};

use crate::knowledge::{normalize, score, view};

/// Proposes candidate entities for an observation.
pub trait Blocker {
    /// Register an entity so it can be proposed later. Called once for every existing entity before
    /// a run, and again whenever the executor creates or extends one — an entity created mid-run
    /// must be findable by the observations that follow it.
    fn index(&mut self, entity: &view::EntityView);

    /// Candidates for one observation, by entity id. Pure: same entity set, same answer.
    fn candidates(&self, observation: &view::ObservationView) -> Vec<i32>;
}

/// Blocks on normalized names and the account a capture showed.
///
/// Both keys are cheap to compute and generous: a shared word order-insensitive name, or a
/// `(scheme, handle)` an observation carries, puts an entity in front of the scorer. Nothing here
/// knows about identity, type or descriptions — those narrow, they do not find.
#[derive(Debug, Default)]
pub struct NameBlocker {
    /// Block key → entity ids. `BTreeSet` so candidates come back in a stable order.
    blocks: HashMap<String, BTreeSet<i32>>,
}

impl NameBlocker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every block key an entity answers to: its own name, each alternative, and its account.
    fn keys_of(entity: &view::EntityView) -> Vec<String> {
        let mut keys: Vec<String> = entity
            .surfaces()
            .flat_map(|surface| [normalize::key(surface), normalize::token_key(surface)])
            .filter(|key| !key.is_empty())
            .collect();
        if let Some((scheme, handle)) = &entity.account {
            keys.push(format!("{scheme}:{}", handle.to_lowercase()));
        }
        keys
    }
}

impl Blocker for NameBlocker {
    fn index(&mut self, entity: &view::EntityView) {
        for key in Self::keys_of(entity) {
            self.blocks.entry(key).or_default().insert(entity.id);
        }
    }

    fn candidates(&self, observation: &view::ObservationView) -> Vec<i32> {
        let observation = score::Observation::from(observation);
        let mut found: BTreeSet<i32> = BTreeSet::new();
        for block in [&observation.name_key, &observation.token_key] {
            if let Some(hits) = self.blocks.get(block) {
                found.extend(hits.iter().copied());
            }
        }
        found.into_iter().collect()
    }
}
