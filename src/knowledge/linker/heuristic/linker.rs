//! The default linker: take the best candidate that clears the bar, otherwise create.
//!
//! A procedure, not a policy. It asks one question —
//! [`Rank::rank`](crate::knowledge::scoring::Rank::rank) — and applies the answer: the
//! best qualifying candidate is where the observation attaches, and no qualifying candidate is a
//! new entity. The bar lives in the policy's `qualifies`, because it is a conjunction over what
//! was compared and a number cannot carry that.

use crate::model;

use crate::knowledge::evidence::{Evidence, IdentityKey, platform_link};
use crate::knowledge::linker::{Action, Decision, Linker};
use crate::knowledge::scoring::Scoring;

use super::Heuristic;

/// Take the highest-scoring candidate that clears the bar; otherwise start a new entity.
///
/// The procedure is fixed; the policy is a member, because a linker decides with one and nothing
/// else.
#[derive(Clone)]
pub struct HeuristicLinker {
    scoring: &'static dyn Scoring,
}

impl std::fmt::Debug for HeuristicLinker {
    /// `Scoring` holds trait objects, so there is nothing to print but the type.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeuristicLinker").finish_non_exhaustive()
    }
}

impl HeuristicLinker {
    /// The heuristic's own policy.
    pub fn new() -> Self {
        Self {
            scoring: Heuristic::scoring(),
        }
    }
}

impl Default for HeuristicLinker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Linker for HeuristicLinker {
    /// Give the batch's observations their entities.
    ///
    /// A held identity settles the question before any scoring: an entity already annotated with this
    /// platform link *is* the thing, and asking a name-similarity model to second-guess that is how a
    /// handle and a real name end up as two entities. Several entities may hold it, so the most
    /// substantiated one wins and the rest are left to the scan, which is where "are these one
    /// thing?" is asked.
    ///
    /// An identity nothing holds yet does *not* settle it: the capture asserted a handle, which is
    /// strong evidence about the handle and weak evidence about the referent. So the observation
    /// falls through to similarity with its identity carried along — a name match may attach it to an
    /// entity that has the name and was missing the handle.
    ///
    /// Each observation is decided on its own, against the corpus as it stood before this
    /// illumination was linked: nothing is set aside for a sibling, and two observations may name the
    /// same entity. Whether that is right is the extraction's question, not the linker's.
    async fn decide_link(
        &self,
        blocker: &dyn crate::knowledge::blocker::Blocker,
        evidence: &Evidence,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<Vec<Decision>> {
        // The order the capture emitted them, which decides who wins a contested entity.
        let mut observations = observations.to_vec();
        observations.sort_by_key(|observation| (observation.position, observation.id));

        let mut decisions = Vec::new();

        for observation in observations {
            if observation.knowledge_entity_id.is_some() {
                continue;
            }
            // A platform link names an identity only if it has a handle; the observation itself has
            // no identity.
            let key =
                platform_link(&observation).and_then(|link| IdentityKey::try_from(&link).ok());
            let holders = match key {
                Some(key) => evidence.query_entities_for_identity(&key).await?,
                None => Vec::new(),
            };

            // The holders arrive most substantiated first; the rest go to the scan.
            if let Some(best) = holders.into_iter().next() {
                decisions.push(Decision {
                    observation,
                    action: Action::Attach(best.id),
                });
                continue;
            }

            let candidates = blocker.candidates(&observation).await;
            let ranked = self.scoring.link(&observation, &candidates);

            // The best candidate that qualifies. An empty ranking is a decision too: nothing here
            // is this observation, so it starts a new entity.
            let action = match ranked.first() {
                Some((index, _)) => Action::Attach(candidates[*index].id),
                None => Action::Create,
            };
            decisions.push(Decision {
                observation,
                action,
            });
        }

        Ok(decisions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::model;
    use crate::test_support::db_or_skip;

    /// Rows that were never persisted: `SqlBlocker` reads `knowledge_blocking_index`, and these stubs have
    /// no rows to read. The linker only needs something that answers candidates.
    struct StubBlocker(Vec<model::knowledge_entity::Model>);

    #[async_trait::async_trait]
    impl crate::knowledge::blocker::Blocker for StubBlocker {
        async fn candidates(
            &self,
            _observation: &model::user_observed_entity::Model,
        ) -> Vec<model::knowledge_entity::Model> {
            self.0.clone()
        }
    }

    fn entity_row(id: i32, name: &str, ty: &str) -> model::knowledge_entity::Model {
        crate::test_support::fixtures::stub_entity(id, name, ty)
    }

    fn observation_row(name: &str, ty: &str) -> model::user_observed_entity::Model {
        crate::test_support::fixtures::stub_observation(1, name, ty)
    }

    /// Link one illumination: build the blocker from the corpus and run the linker over it.
    ///
    /// Needs a database now, because the linker asks the evidence store whether an identity is already
    /// held. Observations with no platform link never reach that call.
    async fn link(
        db: &crate::database::DbHandle,
        entities: &[model::knowledge_entity::Model],
        observations: &[model::user_observed_entity::Model],
    ) -> Vec<Decision> {
        let blocker = StubBlocker(entities.to_vec());
        let evidence = Evidence::from_handle(db);
        HeuristicLinker::new()
            .decide_link(&blocker, &evidence, observations)
            .await
            .expect("the linker should not fail")
    }

    #[tokio::test]
    async fn the_linker_takes_the_best_candidate_that_clears_the_bar() {
        db_or_skip!(db);
        let weak = entity_row(1, "Train Dreams", "book");
        // The same name and type, and a description that agrees with the capture: strictly better.
        let mut strong = entity_row(2, "Train Dreams", "book");
        strong.description = Some("a novella by Denis Johnson".to_string());
        strong.alt_names = vec!["Train Dreams".to_string()];
        let mut observation = observation_row("Train Dreams", "book");
        observation.description = Some("a novella by Denis Johnson".to_string());

        let result = link(&db, &[weak, strong], &[observation]).await;

        match result.as_slice() {
            [decision] => match &decision.action {
                Action::Attach(entity) => {
                    assert_eq!(*entity, 2, "the corroborated candidate wins")
                }
                other => panic!("expected an attach, got {other:?}"),
            },
            other => panic!("expected one decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_candidates_creates_with_nothing_to_report() {
        db_or_skip!(db);
        let result = link(&db, &[], &[observation_row("Anything", "unknown")]).await;
        match result.as_slice() {
            [decision] => match &decision.action {
                Action::Create => {}
                other => panic!("expected a create, got {other:?}"),
            },
            other => panic!("expected one decision, got {other:?}"),
        }
    }
}
