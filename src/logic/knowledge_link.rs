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
    let stats = link(&state.knowledge, &task_run.task)
        .await
        .map_err(api::ApiError::internal)?;

    tracing::info!(
        illumination_id = task_run.task.illumination_id,
        attached = stats.attached,
        attached_by_claim = stats.attached_by_claim,
        created = stats.created,
        discarded_on_conflict = stats.discarded_on_conflict,
        vetoed = stats.vetoed,
        "Knowledge link completed"
    );
    Ok(())
}

/// The work itself: the graph and nothing else, so it can be exercised without a `LogicState`.
pub(crate) async fn link(
    knowledge: &crate::knowledge::Knowledge,
    task: &KnowledgeLinkTask,
) -> anyhow::Result<crate::knowledge::LinkStats> {
    knowledge
        .link_illumination_by_id(task.illumination_id, task.entity_owner_id)
        .await
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

        let stats = link(
            &crate::knowledge::Knowledge::new(
                &db,
                std::sync::Arc::new(crate::knowledge::heuristic::HeuristicLinker),
            ),
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

        let stats = link(
            &crate::knowledge::Knowledge::new(
                &db,
                std::sync::Arc::new(crate::knowledge::heuristic::HeuristicLinker),
            ),
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

        let error = link(
            &crate::knowledge::Knowledge::new(
                &db,
                std::sync::Arc::new(crate::knowledge::heuristic::HeuristicLinker),
            ),
            &task(illumination, stranger),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("not found"), "{error}");

        let linked = model::user_observed_entity::Entity::find()
            .filter(model::user_observed_entity::Column::KnowledgeEntityId.is_not_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(linked, 0, "a rejected task must not write");
    }
}
