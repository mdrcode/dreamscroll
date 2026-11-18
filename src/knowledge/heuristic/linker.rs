//! Today's link policy: a held identity wins; else the best qualifying candidate; else create.

use crate::illumination::v2::PlatformLink;
use crate::model;

use super::HeuristicScorer;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::graph::GraphSelector;
use crate::knowledge::identity::Identity;
use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::knowledge::mutation::GraphMutation;

/// Today's link policy: a held identity wins; else the best qualifying candidate; else create.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicLinker {
    scorer: HeuristicScorer,
}

impl HeuristicLinker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn plan(
        &self,
        blocker: &dyn Blocker,
        selector: &GraphSelector<'_>,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<Vec<GraphMutation>> {
        // The order the capture emitted them, which decides who wins a contested entity.
        let mut observations = observations.to_vec();
        observations.sort_by_key(|observation| (observation.position, observation.id));

        let mut mutations = Vec::with_capacity(observations.len());
        for observation in observations {
            let with = PlatformLink::from_columns(
                observation.platform.clone(),
                observation.platform_handle.clone(),
                observation.platform_display_name.clone(),
            )
            .and_then(|link| Identity::from_platform_link(&link))
            .map(|mut identity| {
                identity.observation_id = Some(ObservationId(observation.id));
                identity
            });

            // A held identity settles it before any scoring. The capture's handle is the observation's
            // own evidence; the entity carrying it is the thing.
            if let Some(identity) = &with
                && let Some(entity) = selector
                    .holder(&identity.scheme, &identity.external_id)
                    .await?
            {
                mutations.push(GraphMutation::AttachObservation {
                    observation: ObservationId(observation.id),
                    entity,
                    with,
                    why: Some("identity holder".into()),
                });
                continue;
            }

            let candidates = blocker.candidates(&observation).await?;
            let ranked = self.scorer.link(&observation, &candidates);

            // The best candidate that qualifies. An empty ranking is a decision too: nothing here
            // is this observation, so it starts a new entity.
            match ranked.first() {
                Some((index, _)) => mutations.push(GraphMutation::AttachObservation {
                    observation: ObservationId(observation.id),
                    entity: origin_of(&candidates[*index])?,
                    with,
                    why: Some("heuristic".into()),
                }),
                None => mutations.push(GraphMutation::CreateEntity {
                    from: ObservationId(observation.id),
                    with,
                }),
            }
        }

        Ok(mutations)
    }
}

fn origin_of(entity: &model::knowledge_entity::Model) -> anyhow::Result<EntityOriginId> {
    Ok(EntityOriginId(ObservationId(entity.origin_observation_id)))
}
