//! Link one illumination into the knowledge graph.
//!
//! One illumination is the natural unit of work: it is what the illuminator finishes, it is what
//! `link_illumination` already takes, and it keeps the retry boundary in the same place as every
//! other queued operation. The graph accumulates reading by reading, so a failed link costs one
//! reading rather than a capture's whole history.
//!
//! The cost to watch: `NameBlocker` loads every entity in the graph, and this task rebuilds it per
//! illumination. That is the price of not sharing mutable state across queue workers, and it is the
//! thing to revisit if linking ever dominates.

use serde::{Deserialize, Serialize};

use crate::{api, task};

/// The concrete task for linking one illumination's observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeLinkTask {
    pub illumination_id: i32,
    pub entity_owner_id: i32,
}

impl task::Task for KnowledgeLinkTask {
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
        "knowledge_link"
    }
}

pub async fn exec(
    state: &super::LogicState,
    task_run: &task::TaskRun<KnowledgeLinkTask>,
    _attempt: i32,
) -> Result<(), api::ApiError> {
    let stats = link(&state.knowledge, &state.service_api, &task_run.task)
        .await
        .map_err(api::ApiError::internal)?;

    // Record whatever this capture made close to something else — the scan only writes rows, so it
    // is safe here and means a duplicate is visible before anyone runs resolution.
    let raised = state
        .knowledge
        .scan_proximities()
        .await
        .map_err(api::ApiError::internal)?
        .raised;

    tracing::info!(
        illumination_id = task_run.task.illumination_id,
        attached = stats.attached,
        created = stats.created,
        proximities_raised = raised,
        "Knowledge link completed"
    );
    Ok(())
}

/// The work itself: the graph and nothing else, so it can be exercised without a `LogicState`.
///
/// This is where the payload is checked against the world. A task names its owner and an
/// illumination row has one, but the graph does not: entities are shared by everyone who observed
/// them, so an owner-scoped read belongs to the caller that was handed both ids.
pub(crate) async fn link(
    knowledge: &crate::knowledge::Knowledge,
    service_api: &crate::api::ServiceApiClient,
    task: &KnowledgeLinkTask,
) -> anyhow::Result<crate::knowledge::LinkStats> {
    // The read belongs to the service layer, which is the worker's door to the app; the ownership
    // check belongs here, where the payload is. The store is not involved in either.
    let illumination = service_api.illumination(task.illumination_id).await?;
    anyhow::ensure!(
        illumination.user_id == task.entity_owner_id,
        "illumination {} belongs to user {}, not {}",
        illumination.id,
        illumination.user_id,
        task.entity_owner_id
    );

    knowledge.link_illumination(&illumination).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::prelude::*;

    use crate::model;
    use crate::test_support::{fixtures, test_db};

    fn task(illumination_id: i32, entity_owner_id: i32) -> KnowledgeLinkTask {
        KnowledgeLinkTask {
            illumination_id,
            entity_owner_id,
        }
    }

    #[tokio::test]
    async fn linking_one_illumination_accumulates_into_the_graph() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "knowledge_link").await;
        let capture = fixtures::capture(&db, user).await;

        let first = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, first, 0, "Train Dreams").await;

        let service_api = crate::test_support::service_api(&db);
        let stats = link(
            &crate::knowledge::Knowledge::with_linking(
                &db,
                Box::new(
                    crate::knowledge::name_blocker::NameBlocker::new(&db)
                        .await
                        .unwrap(),
                ),
                std::sync::Arc::new(crate::knowledge::heuristic_linker::HeuristicLinker),
            ),
            &service_api,
            &task(first, user),
        )
        .await
        .unwrap();
        assert_eq!(stats.created, 1);
        assert_eq!(stats.attached, 0);

        // The capture is read again and names the same thing: the second reading attaches to the
        // entity the first one created, which is the whole point of linking one illumination at a time.
        let second = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, second, 0, "Train Dreams").await;

        let service_api = crate::test_support::service_api(&db);
        let stats = link(
            &crate::knowledge::Knowledge::with_linking(
                &db,
                Box::new(
                    crate::knowledge::name_blocker::NameBlocker::new(&db)
                        .await
                        .unwrap(),
                ),
                std::sync::Arc::new(crate::knowledge::heuristic_linker::HeuristicLinker),
            ),
            &service_api,
            &task(second, user),
        )
        .await
        .unwrap();
        assert_eq!(stats.created, 0);
        assert_eq!(stats.attached, 1);

        let entities = model::knowledge_entity::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        assert_eq!(
            entities.len(),
            1,
            "two readings of one thing are one entity"
        );
        assert_eq!(entities[0].name, "Train Dreams");
        assert!(entities[0].origin_observation_id.is_some());
    }

    #[tokio::test]
    async fn an_illumination_belonging_to_another_user_is_not_linked() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let owner = fixtures::user(&db, "knowledge_link_owner").await;
        let stranger = fixtures::user(&db, "knowledge_link_stranger").await;
        let capture = fixtures::capture(&db, owner).await;
        let illumination = fixtures::illumination(&db, owner, capture).await;
        fixtures::entity(&db, owner, capture, illumination, 0, "Train Dreams").await;

        let service_api = crate::test_support::service_api(&db);
        let error = link(
            &crate::knowledge::Knowledge::with_linking(
                &db,
                Box::new(
                    crate::knowledge::name_blocker::NameBlocker::new(&db)
                        .await
                        .unwrap(),
                ),
                std::sync::Arc::new(crate::knowledge::heuristic_linker::HeuristicLinker),
            ),
            &service_api,
            &task(illumination, stranger),
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("belongs to user"),
            "the task must refuse a payload naming another user: {error}"
        );

        let linked = model::user_observed_entity::Entity::find()
            .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(linked, 0, "a rejected task must not write");
    }
}
