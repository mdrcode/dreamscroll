//! The knowledge graph's write side: [`Knowledge`].
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`Knowledge`], and the handle itself never leaves it — the primitives below
//! are private, and the three collaborators (`decider`, `blocker`, `evidence`) are `pub(super)` only
//! because the linking code that reads them lives in `link.rs`. Outside `knowledge`, a caller sees
//! [`Knowledge`]'s methods and nothing else.

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
/// [`Knowledge::with_linking`].
///
/// The blocker is built once and kept, so it accumulates everything this store writes — `project`
/// tells it about every entity whose columns changed, and a discarded or tombstoned entity is
/// forgotten. It therefore knows *this* store's writes only: a second writer (the CLI, another
/// instance) is invisible to it. That is the argument for a blocker that queries instead of holding
/// an index, not for rebuilding this one per call.
#[derive(Clone)]
pub struct Knowledge {
    db: DbHandle,
    pub(super) decider: Option<Arc<dyn LinkDecider>>,
    /// `Arc` because `Knowledge` is `Clone` and the blocker is shared by every writer on it. Held
    /// rather than rebuilt so it accumulates everything this store writes. No lock here: the
    /// blocker's methods take `&self` and the implementation synchronizes itself, so a guard never
    /// outlives one call — which is what used to make a notification during a link deadlock.
    pub(super) blocker: Option<Arc<dyn Blocker>>,
    /// The claims about entities. Built here rather than per link: it is a handle, not state, and a
    /// caller that links thousands of illuminations should not be constructing one each time.
    pub(super) evidence: Evidence,
}

impl Knowledge {
    /// The graph, for reads and writes. Linking on it fails; see [`Self::with_linking`].
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
    pub fn with_linking(
        db: &DbHandle,
        blocker: Box<dyn Blocker>,
        decider: Arc<dyn LinkDecider>,
    ) -> Self {
        Self {
            decider: Some(decider),
            blocker: Some(blocker.into()),
            ..Self::new(db)
        }
    }

    /// Tell the held blocker what just happened, if there is one.
    fn notify(&self, entity: &model::knowledge_entity::Model) {
        if let Some(blocker) = &self.blocker {
            blocker.changed(entity);
        }
    }

    /// Tell the blocker an entity is gone. `pub(super)` so `merge` can retire the loser.
    pub(super) fn forget(&self, knowledge_entity_id: i32) {
        if let Some(blocker) = &self.blocker {
            blocker.forget(knowledge_entity_id);
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
        self.forget(knowledge_entity_id);
        Ok(())
    }

    /// Link one reading into the graph: cluster its observations, record their claims, and project
    /// every entity it touched.
    ///
    /// The implementation is in [`super::link`]; it is here so the facade reads as one list of what a
    /// caller can ask the graph to do.
    pub async fn link_illumination(
        &self,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<super::LinkStats> {
        super::link::link_illumination(self, illumination).await
    }

    /// Recompute an entity's projected columns, and tell the blocker what came out.
    ///
    /// The notification lives with this method rather than in the linker's loop because projection is
    /// the one place those columns change: linking, resolving and merging all end up here, so a held
    /// blocker cannot miss one of them. The derivation itself is [`super::project`].
    pub async fn update_projection(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        let entity = super::project::project(&self.db.conn, knowledge_entity_id).await?;
        self.notify(&entity);
        Ok(entity)
    }

    /// What a merge would do, without doing it. See [`super::merge`].
    pub async fn merge_preview(
        &self,
        loser: i32,
        winner: i32,
    ) -> anyhow::Result<super::merge::Preview> {
        super::merge::preview(&self.db.conn, loser, winner).await
    }

    /// Fold `loser` into `winner`. See [`super::merge`].
    pub async fn merge(&self, loser: i32, winner: i32) -> anyhow::Result<super::merge::Outcome> {
        super::merge::merge_into(&self.db.conn, self, loser, winner).await
    }
}

// ---------------------------------------------------------------------------------------------
// The implementation. Connection-generic, so a caller inside a transaction can share it.
// ---------------------------------------------------------------------------------------------

/// This is the only place a `knowledge_entity_id` is set. `project` reads the resulting membership
/// to compute the entity's columns, so nothing else about the move is recorded here.
async fn attach<C: ConnectionTrait>(
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

async fn insert_new<C: ConnectionTrait>(
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

async fn fetch<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let found = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    found.ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))
}

async fn discard_empty<C: ConnectionTrait>(
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
