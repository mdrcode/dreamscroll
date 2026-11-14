//! Resolve the identities of the entities one illumination touched.
//!
//! Chained after the link: the link created or attached the entities, and this asks the identity
//! resolvers about exactly those. The payload is the illumination, not its entity ids, because entity
//! ids are generation-scoped — a task delivered after a `reset` would name rows that no longer exist,
//! or different rows — while the illumination is stable and its entity set is derived at execution.
//!
//! The run is bounded: one attempt may make [`AllowanceConfig::max_calls`] transport calls at
//! [`AllowanceConfig::min_interval`]. A refusal (budget spent, provider rate limit) is a completed run with a
//! report, not an error: the entity stays pending, and the batch job
//! (`plan/knowledge-batch-job.md`) is the floor that reads it again.

use serde::{Deserialize, Serialize};

use crate::knowledge::identity::IdentityEngine;
use crate::{api, knowledge, task};

/// The concrete task for resolving one illumination's entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityResolveTask {
    pub illumination_id: i32,
    pub entity_owner_id: i32,
}

impl task::Task for IdentityResolveTask {
    fn entity_type() -> &'static str {
        "illumination"
    }

    fn entity_id(&self) -> i32 {
        self.illumination_id
    }

    fn entity_owner_id(&self) -> i32 {
        self.entity_owner_id
    }

    fn task_type() -> &'static str {
        "identity_resolve"
    }
}

pub async fn exec(
    state: &super::LogicState,
    task_run: &task::TaskRun<IdentityResolveTask>,
    _attempt: i32,
) -> Result<(), api::ApiError> {
    let report = resolve(
        &state.identity_engine,
        &state.cache,
        &state.service_api,
        &task_run.task,
        state.wikidata_allowance,
    )
    .await
    .map_err(api::ApiError::internal)?;

    tracing::info!(
        illumination_id = task_run.task.illumination_id,
        considered = report.considered,
        resolved = report.entities_resolved,
        written = report.identities_written,
        replaced = report.identities_replaced,
        vetoed = report.identities_vetoed,
        refused = report.refusal.is_some(),
        "KnowledgeGraph identity resolve completed"
    );
    Ok(())
}

/// The work itself: the graph and the resolver, so it can be exercised without a queue.
///
/// The ownership check belongs here, where the payload is: an illumination row names its user, and
/// the task claims to be acting for one.
pub(crate) async fn resolve(
    engine: &IdentityEngine,
    cache: &crate::fetch::FetchCache,
    service_api: &crate::api::ServiceApiClient,
    task: &IdentityResolveTask,
    allowance: crate::fetch::AllowanceConfig,
) -> anyhow::Result<knowledge::identity::ResolutionReport> {
    let illumination = service_api.illumination(task.illumination_id).await?;
    anyhow::ensure!(
        illumination.user_id == task.entity_owner_id,
        "illumination {} belongs to user {}, not {}",
        illumination.id,
        illumination.user_id,
        task.entity_owner_id
    );

    // The read refuses an illumination whose link has not finished, so the batch is always the
    // linked set; resolving a partial one would silently drop the unlinked observations.
    let entities = service_api.illumination_entities(task.illumination_id).await?;

    let allowance = crate::fetch::Allowance::configured(allowance);
    engine.resolve(&entities, cache, Some(&allowance)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use crate::knowledge::KnowledgeGraph;
    use crate::test_support::{fixtures, service_api, test_db};

    #[test]
    fn the_payload_round_trips_as_a_task() {
        let task = IdentityResolveTask {
            illumination_id: 7,
            entity_owner_id: 3,
        };
        let run = task::TaskRun::new(task, 1);
        assert_eq!(run.logical_id, "u3-identity_resolve-illumination7");
        let wire = serde_json::to_string(&run).expect("serializes");
        let back: task::TaskRun<IdentityResolveTask> =
            serde_json::from_str(&wire).expect("deserializes");
        assert_eq!(back.task.illumination_id, 7);
        assert_eq!(back.task.entity_owner_id, 3);
        assert_eq!(back.logical_id, run.logical_id);
    }

    /// The payload names an owner; an illumination row names its user. Acting for someone else is
    /// refused before any transport call.
    struct Empty;

    #[async_trait::async_trait]
    impl crate::knowledge::identity::IdentityResolver for Empty {
        fn id(&self) -> &'static str {
            "fake"
        }

        async fn resolve(
            &self,
            _entities: &[crate::model::knowledge_entity::Model],
            _gateway: &crate::fetch::Gateway<'_>,
        ) -> Vec<(i32, crate::knowledge::evidence::Identity)> {
            Vec::new()
        }
    }

    fn engine(graph: &knowledge::KnowledgeGraph) -> IdentityEngine {
        IdentityEngine::new(
            graph.clone(),
            crate::knowledge::identity::Options::default(),
            vec![Box::new(Empty)],
        )
        .expect("the fake provider is not reserved")
    }

    #[tokio::test]
    async fn a_resolve_for_another_users_illumination_is_refused() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "identity_resolve").await;
        let other = fixtures::user(&db, "identity_resolve_other").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;

        let graph = KnowledgeGraph::new(&db);
        let cache = crate::fetch::FetchCache::new(&db.conn);
        let error = resolve(
            &engine(&graph),
            &cache,
            &service_api(&db),
            &IdentityResolveTask {
                illumination_id: illumination,
                entity_owner_id: other,
            },
            crate::fetch::AllowanceConfig {
                max_calls: 0,
                min_interval: Duration::ZERO,
            },
        )
        .await
        .expect_err("owner mismatch must be refused");

        assert!(error.to_string().contains("belongs to user"), "{error}");
    }

    /// The whole path, with no transport calls: link creates the entity, the resolve considers it,
    /// and a zero budget means the provider is never asked (so nothing is annotated).
    #[tokio::test]
    async fn a_zero_budget_resolve_considers_the_entities_and_asks_nothing() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "identity_resolve_budget").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        let graph = KnowledgeGraph::with_heuristic_linking(&db);
        let api = service_api(&db);
        crate::logic::knowledge_link::link(
            &graph,
            &api,
            &crate::logic::knowledge_link::KnowledgeLinkTask {
                illumination_id: illumination,
                entity_owner_id: user,
            },
        )
        .await
        .expect("link");

        let cache = crate::fetch::FetchCache::new(&db.conn);
        let report = resolve(
            &engine(&graph),
            &cache,
            &api,
            &IdentityResolveTask {
                illumination_id: illumination,
                entity_owner_id: user,
            },
            crate::fetch::AllowanceConfig {
                max_calls: 0,
                min_interval: Duration::ZERO,
            },
        )
        .await
        .expect("resolve");

        assert_eq!(report.considered, 1, "{report:?}");
        assert_eq!(report.entities_resolved, 0, "{report:?}");
        assert_eq!(report.identities_written, 0, "{report:?}");
    }

    #[tokio::test]
    async fn a_resolve_refuses_an_illumination_whose_link_has_not_run() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "identity_resolve_unlinked").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        let graph = KnowledgeGraph::new(&db);
        let cache = crate::fetch::FetchCache::new(&db.conn);
        let error = resolve(
            &engine(&graph),
            &cache,
            &service_api(&db),
            &IdentityResolveTask {
                illumination_id: illumination,
                entity_owner_id: user,
            },
            crate::fetch::AllowanceConfig {
                max_calls: 10,
                min_interval: Duration::ZERO,
            },
        )
        .await
        .expect_err("an unlinked illumination must be refused");

        assert!(error.to_string().contains("unlinked"), "{error}");
    }
}
