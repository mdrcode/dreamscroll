//! The knowledge graph's write side: [`Knowledge`].
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`Knowledge`]. The connection-generic functions at the bottom are the
//! implementation, kept `pub(super)` so `merge` can run them inside its transaction; nothing outside
//! this module can reach a connection at all.

use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

use crate::database::DbHandle;
use crate::model;

use super::blocker::Blocker;
use super::evidence::Evidence;
use super::link_decider::LinkDecider;

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

/// The knowledge graph, as seen by anyone who writes to it.
///
/// Most callers are a store: they create, attach, project and merge, and never decide anything. Those
/// build [`Knowledge::new`]. Linking additionally needs a policy and a blocker, both injected by
/// [`Knowledge::with_linker`].
///
/// The blocker is built once and kept, so it accumulates everything this store writes — `project`
/// tells it about every entity whose columns changed, and a discarded or tombstoned entity is
/// forgotten. It therefore knows *this* store's writes only: a second writer (the CLI, another
/// instance) is invisible to it. That is the argument for a blocker that queries instead of holding
/// an index, not for rebuilding this one per call.
#[derive(Clone)]
pub struct Knowledge {
    /// `pub(super)`: `project` and `merge` live in their own files and write through this handle.
    pub(super) db: DbHandle,
    pub(super) decider: Option<Arc<dyn LinkDecider>>,
    /// `Mutex` because `Blocker::changed` takes `&mut`; `Arc` because `Knowledge` is `Clone`. Held
    /// rather than rebuilt so it accumulates everything this store writes.
    pub(super) blocker: Option<Arc<tokio::sync::Mutex<Box<dyn Blocker>>>>,
    /// The claims about entities. Built here rather than per link: it is a handle, not state, and a
    /// caller that links thousands of illuminations should not be constructing one each time.
    pub(super) evidence: Evidence,
}

impl Knowledge {
    /// The graph, for reads and writes. Linking on it fails; see [`Self::with_linker`].
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            decider: None,
            blocker: None,
            evidence: Evidence::from_handle(db),
        }
    }

    /// The graph, with the policy that decides how a reading's observations cluster, and the index
    /// it asks. `NameBlocker::new(&db)` is the usual blocker.
    pub fn with_linker(
        db: &DbHandle,
        decider: Arc<dyn LinkDecider>,
        blocker: Box<dyn Blocker>,
    ) -> Self {
        Self {
            decider: Some(decider),
            blocker: Some(Arc::new(tokio::sync::Mutex::new(blocker))),
            ..Self::new(db)
        }
    }

    /// Tell the held blocker what just happened, if there is one.
    /// `pub(super)` for the same reason as `db`: projection lives in `project.rs`.
    pub(super) async fn notify(&self, entity: &model::knowledge_entity::Model) {
        if let Some(blocker) = &self.blocker {
            blocker.lock().await.changed(entity);
        }
    }

    /// Tell the blocker an entity is gone. `pub(super)` so `merge` can retire the loser.
    pub(super) async fn forget(&self, knowledge_entity_id: i32) {
        if let Some(blocker) = &self.blocker {
            blocker.lock().await.forget(knowledge_entity_id);
        }
    }

    /// The entity, or an error. Every caller that asks has already decided it exists.
    pub async fn get(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        fetch(&self.db.conn, knowledge_entity_id).await
    }

    pub async fn create(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        insert_new(&self.db.conn, observation).await
    }

    /// Point an observation at the entity it belongs to.
    pub async fn attach_observation(
        &self,
        observation_id: i32,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<()> {
        attach(&self.db.conn, observation_id, knowledge_entity_id).await
    }

    /// Throw away an entity that owns nothing — a create that lost its claim to another entity.
    ///
    /// Named for the predicate, and the predicate is checked: an entity that still has members is
    /// refused rather than deleted. The foreign key would fail the delete anyway, but only after the
    /// fact and with a message about constraints rather than about this call.
    pub async fn discard_empty(&self, knowledge_entity_id: i32) -> anyhow::Result<()> {
        discard_empty(&self.db.conn, knowledge_entity_id).await?;
        self.forget(knowledge_entity_id).await;
        Ok(())
    }

    /// The connection the graph writes with, for the one caller that has to read a row before it can
    /// ask for a write: the link task, which is told "link illumination 7" and must load it.
    ///
    /// Every *write* has a method here, and everything inside `knowledge/` reaches the handle through
    /// the field rather than through this. If a second outside caller appears, that is the signal that
    /// the read it needs belongs as a method instead.
    pub fn conn(&self) -> &sea_orm::DatabaseConnection {
        &self.db.conn
    }
}

// ---------------------------------------------------------------------------------------------
// The implementation. Connection-generic, so a caller inside a transaction can share it.
// ---------------------------------------------------------------------------------------------

/// This is the only place a `knowledge_entity_id` is set. `project` reads the resulting membership
/// to compute the entity's columns, so nothing else about the move is recorded here.
pub(super) async fn attach<C: ConnectionTrait>(
    conn: &C,
    observation_id: i32,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    model::user_observed_entity::Entity::update_many()
        .col_expr(
            model::user_observed_entity::Column::KnowledgeEntityId,
            Expr::value(Some(knowledge_entity_id)),
        )
        .filter(model::user_observed_entity::Column::Id.eq(observation_id))
        .exec(conn)
        .await?;
    Ok(())
}

pub(super) async fn insert_new<C: ConnectionTrait>(
    conn: &C,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<model::knowledge_entity::Model> {
    // Always provisional: a new entity has nothing proving it yet. If the capture asserted a claim,
    // it is recorded against this entity and `project` promotes it to `identified` moments later —
    // guessing the status here would be a second writer of a projected column.
    let model = model::knowledge_entity::ActiveModel {
        name: Set(observation.name.trim().to_string()),
        entity_type: Set(observation.entity_type.clone()),
        alt_types: Set(Vec::new()),
        description: Set(observation.description.clone()),
        alt_descriptions: Set(Vec::new()),
        status: Set(STATUS_PROVISIONAL.to_string()),
        confidence: Set(None),
        merged_into_id: Set(None),
        alt_names: Set(Vec::new()),
        // Where the entity came from, recorded once. Projection wears this member's values and
        // treats every other member as an alternative, so losing it silently demotes the entity to
        // "whatever member happens to be first".
        origin_observation_id: Set(Some(observation.id)),
        ..Default::default()
    };
    Ok(model::knowledge_entity::Entity::insert(model)
        .exec_with_returning(conn)
        .await?)
}

pub(super) async fn fetch<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let found = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    found.ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))
}

pub(super) async fn discard_empty<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    let members = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .count(conn)
        .await?;
    anyhow::ensure!(
        members == 0,
        "knowledge entity {knowledge_entity_id} still owns {members} observation(s); it is not empty \
         and must not be discarded"
    );
    model::knowledge_entity::Entity::delete_by_id(knowledge_entity_id)
        .exec(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{fixtures, test_db};

    /// The name is the contract: an entity that still owns observations is refused, not deleted.
    /// (The foreign key would fail it too — but on a call that was never meant to reach the database.)
    #[tokio::test]
    async fn an_entity_with_members_is_not_discarded() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;

        let error = Knowledge::new(&db).discard_empty(id).await.unwrap_err();
        assert!(error.to_string().contains("still owns"), "{error}");

        assert_eq!(
            Knowledge::new(&db).get(id).await.unwrap().id,
            id,
            "the row must survive"
        );
    }
}
