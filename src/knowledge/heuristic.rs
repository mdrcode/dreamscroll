//! The default linker: take the best candidate that clears the bar, otherwise create.
//!
//! A procedure, not a policy. Every number it uses comes from the [`Scoring`] it is handed — the
//! name similarity and weights from [`Similarity`], the bar from [`Judge::same`], and the ordering
//! of identity holders from [`Worth::order`]. Swap the policy and this file does not change; write a
//! different procedure (an embedding recall, a model's judgement) and it implements [`LinkDecider`]
//! instead.
//!
//! A name alone never decides: the bar requires corroboration, and that bar is the policy's.

use crate::model;

use super::evidence::Evidence;
use super::link_decider::{Action, Decision, LinkDecider};
use super::score::{Scoring, Subject};

/// Take the highest-scoring candidate that clears the bar; otherwise start a new entity.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicLinker;

#[async_trait::async_trait]
impl LinkDecider for HeuristicLinker {
    /// Give the illumination's observations their entities.
    ///
    /// A held identity settles the question before any scoring: an entity that already claims this
    /// platform link *is* the thing, and asking a name-similarity model to second-guess that is how a
    /// handle and a real name end up as two entities. Several entities may hold it, so the most
    /// substantiated one wins and the rest are left to reconcile, which is where "are these one
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
        blocker: &dyn super::blocker::Blocker,
        evidence: &Evidence,
        scoring: &Scoring,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<Vec<Decision>> {
        // An unloaded relation would read as an illumination with no observations, and linking
        // nothing would look exactly like success.
        if !illumination.observations.is_loaded() {
            anyhow::bail!(
                "the illumination's observations are not loaded; fetch it with Entity::load().with(…) "
            );
        }
        // `ModelEx` rather than `Model`: the relation is loaded, and the linker wants plain rows.
        let mut observations: Vec<model::user_observed_entity::Model> = illumination
            .observations
            .iter()
            .map(|row| row.clone().into())
            .collect();
        // The order the capture emitted them, which decides who wins a contested entity.
        observations.sort_by_key(|observation| (observation.position, observation.id));

        let mut decisions = Vec::new();

        for observation in observations {
            if observation.knowledge_entity_id.is_some() {
                continue;
            }
            // Already a row: the executor imported this illumination's identities before deciding.
            let identity = evidence.identity_of(&observation).await?;

            // `held_by_identity` returns the most substantiated first; the rest go to reconcile.
            if let Some(identity_id) = identity
                && let Some(best) = evidence
                    .held_by_identity(identity_id, &*scoring.worth)
                    .await?
                    .into_iter()
                    .next()
            {
                decisions.push(Decision {
                    observation,
                    action: Action::Attach(best.id),
                    identity,
                });
                continue;
            }

            let candidates = blocker.candidates(&observation).await;
            let features = scoring
                .similarity
                .fit(&Subject::of_observation(&observation), &candidates);

            // (entity id, total, clears the bar): highest total wins, and whether it is worth
            // merging decides between attaching and creating. Ties keep the first candidate, i.e.
            // the lowest entity id, because the blocker hands them back in id order.
            let mut best: Option<(i32, f32, bool)> = None;
            for (candidate, features) in candidates.iter().zip(&features) {
                let entry = (candidate.id, features.total, scoring.judge.same(features));
                if best.is_none_or(|winner| entry.1 > winner.1) {
                    best = Some(entry);
                }
            }

            let action = match best {
                Some((entity, _, true)) => Action::Attach(entity),
                _ => Action::Create,
            };
            decisions.push(Decision {
                observation,
                action,
                identity,
            });
        }

        Ok(decisions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::model;

    use super::super::name_blocker::NameBlocker;

    fn entity_row(id: i32, name: &str, ty: &str) -> model::knowledge_entity::Model {
        crate::test_support::fixtures::stub_entity(id, name, ty)
    }

    fn observation_row(name: &str, ty: &str) -> model::user_observed_entity::Model {
        crate::test_support::fixtures::stub_observation(1, name, ty)
    }

    /// Link one illumination: build the blocker from the corpus and run the linker over it.
    ///
    /// Needs a database now, because the linker asks the evidence store whether a claim is already
    /// held. Observations with no platform link never reach that call.
    async fn link(
        db: &crate::database::DbHandle,
        entities: &[model::knowledge_entity::Model],
        observations: &[model::user_observed_entity::Model],
    ) -> Vec<Decision> {
        let blocker = NameBlocker::build_entities(entities);
        let evidence = Evidence::from_handle(db);
        evidence
            .import_identities(observations)
            .await
            .expect("importing the assertions should not fail");
        let mut illumination = crate::test_support::fixtures::stub_illumination(1);
        illumination.observations = sea_orm::compound::HasMany::Loaded(
            observations.iter().map(|row| row.clone().into()).collect(),
        );
        HeuristicLinker
            .decide_link(
                &blocker,
                &evidence,
                &Scoring::heuristic(),
                &illumination,
            )
            .await
            .expect("the linker should not fail")
    }

    #[tokio::test]
    async fn the_linker_takes_the_best_candidate_that_clears_the_bar() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let weak = entity_row(1, "Train Dreams", "book");
        let strong = entity_row(2, "Train Dreams", "book");
        let observation = observation_row("Train Dreams", "book");

        let result = link(&db, &[weak, strong], &[observation]).await;

        match result.as_slice() {
            [decision] => match &decision.action {
                Action::Attach(entity) => assert!(*entity == 1 || *entity == 2),
                other => panic!("expected an attach, got {other:?}"),
            },
            other => panic!("expected one decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_candidates_creates_with_nothing_to_report() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
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
