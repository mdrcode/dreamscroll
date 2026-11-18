//! Today's link policy: the best qualifying blocker candidate, else create. A handle is carried as
//! an identity but does not bypass the ranking.

use crate::illumination::v2::PlatformLink;
use crate::model;

use super::HeuristicScorer;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::identity::Identity;
use crate::knowledge::ids::{EntityOriginId, ObservationId};
use crate::knowledge::mutation::GraphMutation;

/// Today's link policy: the best qualifying blocker candidate, else create. A handle does not
/// bypass the ranking; it is carried on the resulting attach/create.
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
