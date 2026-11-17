//! Today's policy: a held identity wins; else the best qualifying candidate; else create.

use crate::model;

use crate::knowledge2::action::KnowledgeAction;
use crate::knowledge2::blocker::Blocker;
use crate::knowledge2::identity::Identity;
use crate::knowledge2::ids::{EntityOriginId, ObservationId};
use crate::knowledge2::linker::{IdentityHolder, Linker};
use super::Heuristic;

/// Today's policy: a held identity wins; else the best qualifying candidate; else create.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicPlanner {
    scoring: Heuristic,
}

impl HeuristicPlanner {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Linker for HeuristicPlanner {
    async fn plan(
        &self,
        blocker: &dyn Blocker,
        holder: &dyn IdentityHolder,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<Vec<KnowledgeAction>> {
        // The order the capture emitted them, which decides who wins a contested entity.
        let mut observations = observations.to_vec();
        observations.sort_by_key(|observation| (observation.position, observation.id));

        let mut actions = Vec::with_capacity(observations.len());
        for observation in observations {
            let with = Identity::from_observation(&observation);

            // A held identity settles it before any scoring. The capture's handle is the observation's
            // own evidence; the entity carrying it is the thing.
            if let Some(identity) = &with
                && let Some(entity) = holder
                    .holder(&identity.scheme, &identity.external_id)
                    .await?
            {
                actions.push(KnowledgeAction::AttachObservation {
                    observation: ObservationId(observation.id),
                    entity,
                    with,
                    why: Some("identity holder".into()),
                });
                continue;
            }

            let candidates = blocker.candidates(&observation).await?;
            let ranked = self.scoring.link(&observation, &candidates);

            // The best candidate that qualifies. An empty ranking is a decision too: nothing here
            // is this observation, so it starts a new entity.
            match ranked.first() {
                Some((index, _)) => actions.push(KnowledgeAction::AttachObservation {
                    observation: ObservationId(observation.id),
                    entity: origin_of(&candidates[*index])?,
                    with,
                    why: Some("heuristic".into()),
                }),
                None => actions.push(KnowledgeAction::CreateEntity {
                    from: ObservationId(observation.id),
                    with,
                }),
            }
        }

        Ok(actions)
    }
}

fn origin_of(entity: &model::knowledge_entity::Model) -> anyhow::Result<EntityOriginId> {
    let origin = entity.origin_observation_id.ok_or_else(|| {
        anyhow::anyhow!(
            "blocker returned entity {} with no origin_observation_id",
            entity.id
        )
    })?;
    Ok(EntityOriginId(ObservationId(origin)))
}
