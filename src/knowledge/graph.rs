//! The knowledge graph's write side: [`Knowledge`].
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`Knowledge`], and the handle itself never leaves it — the primitives below
//! are private, and the collaborators (`decider`, `blocker`, `evidence`, `scoring`) are `pub(super)`
//! only because the code that reads them lives in `link.rs`, `project.rs` and `reconcile.rs`.
//! Outside `knowledge`, a caller sees [`Knowledge`]'s methods and nothing else.

use std::sync::Arc;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Set, Statement};

use crate::database::DbHandle;
use crate::model;

use super::blocker::Blocker;
use super::evidence::Evidence;
use super::link_decider::LinkDecider;
use super::scoring::Scoring;

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

/// The knowledge graph, as seen by anyone who writes to it.
///
/// Most callers are a store: they create, attach, project and merge, and never decide anything. Those
/// build [`Knowledge::new`]. Linking additionally needs a policy and a blocker, both injected by
/// [`Knowledge::with_linking`].
///
/// The [`Scoring`] is the store's, not the linker's: `reconcile` asks it whether two entities are one
/// thing and how alike they are, and the linker asks which candidate a capture belongs to. One policy
/// for both, or the graph disagrees with itself. [`Knowledge::new`] uses the heuristic;
/// [`Knowledge::with_scoring`] replaces it.
///
/// The blocker is built once and kept, so it accumulates everything this store writes — `project`
/// tells it about every entity whose columns changed, and a discarded or tombstoned entity is
/// forgotten. It therefore knows *this* store's writes only: a second writer (the CLI, another
/// instance) is invisible to it. That is the argument for a blocker that queries instead of holding
/// an index, not for rebuilding this one per call.
#[derive(Clone)]
pub struct Knowledge {
    db: DbHandle,
    pub(super) blocker: Option<Arc<dyn Blocker>>,
    pub(super) decider: Option<Arc<dyn LinkDecider>>,
    pub(super) evidence: Evidence,
    pub(super) scoring: Scoring,
}

impl Knowledge {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            blocker: None,
            decider: None,
            evidence: Evidence::from_handle(db),
            scoring: Scoring::heuristic(),
        }
    }

    /// Replace the scoring policy. Everything that measures — the linker, the projection, the merge
    /// pass, the resolver — reads this one, so there is nothing else to swap.
    pub fn with_scoring(mut self, scoring: Scoring) -> Self {
        self.scoring = scoring;
        self
    }


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

    /// Tell the blocker an entity is gone. `pub(super)` so `merge` can retire the loser.
    pub(super) fn forget(&self, knowledge_entity_id: i32) {
        if let Some(blocker) = &self.blocker {
            blocker.forget(knowledge_entity_id);
        }
    }

    pub async fn create(
        &self,
        observation: &model::user_observed_entity::Model,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        insert_new(&self.db.conn, observation).await
    }

    pub async fn get(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        get(&self.db.conn, knowledge_entity_id).await
    }

    /// Point a user's observation to the Knowledge entity it belongs to.
    pub async fn attach_observation(
        &self,
        observation_id: i32,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<()> {
        attach(&self.db.conn, observation_id, knowledge_entity_id).await
    }

    /// Link one reading into the graph: cluster its observations, record their claims, and project
    /// every entity it touched.
    ///
    /// Implemented in [`super::link`]
    pub async fn link_illumination(
        &self,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<super::LinkStats> {
        super::link::link_illumination(self, illumination).await
    }

    /// Recompute an entity's projected columns, and tell the blocker what came out.
    ///
    /// Every change to those columns funnels through here, so the held blocker hears about all of
    /// them. Implemented in [`super::project`].
    pub async fn update_projection(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        let entity = super::project::project(&self.db.conn, knowledge_entity_id).await?;
        if let Some(blocker) = &self.blocker {
            blocker.changed(&entity);
        }
        Ok(entity)
    }

    /// Record every pair that shares an identity. Reads the graph, writes only proximities.
    pub async fn scan_proximities(&self) -> anyhow::Result<super::reconcile::Report> {
        super::reconcile::scan(&self.scoring, &self.db.conn).await
    }

    /// Fold the open proximities that look like one thing, and leave the rest as questions.
    pub async fn merge_proximities(&self) -> anyhow::Result<super::reconcile::Report> {
        super::reconcile::merge_open(self, &self.db.conn).await
    }

    /// Fold the holders of each identity that look like one thing, and queue the refusals.
    ///
    /// The pass that answers "these two share an identity — are they the same thing?", which sharing
    /// an identity no longer answers on its own. See [`super::reconcile`].
    pub async fn reconcile(&self) -> anyhow::Result<super::reconcile::Report> {
        super::reconcile::reconcile(self, &self.db.conn).await
    }

    /// Settle the open proximities between two entities with a verdict — `distinct` when a person
    /// says they are two things, which no automatic pass can conclude. Returns how many rows closed.
    pub async fn settle(&self, a: i32, b: i32, verdict: &str) -> anyhow::Result<u64> {
        let result = self
            .db
            .conn
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE knowledge_proximities SET verdict = $1, settled_at = now() \
                  WHERE verdict IS NULL \
                    AND ((holder_id = $2 AND counterpart_id = $3) \
                      OR (holder_id = $3 AND counterpart_id = $2))",
                [verdict.into(), a.into(), b.into()],
            ))
            .await?;
        Ok(result.rows_affected())
    }

    /// What a merge would do, without doing it. See [`super::merge`].
    pub async fn merge_preview(
        &self,
        loser: i32,
        winner: i32,
    ) -> anyhow::Result<super::merge::Preview> {
        super::merge::preview(&self.db.conn, loser, winner).await
    }

    /// Fold `loser` into `winner`: the loser's observations re-point, the loser becomes a tombstone.
    /// See [`super::merge`].
    pub async fn merge(
        &self,
        loser: i32,
        winner: i32,
        cause: super::merge::Cause,
    ) -> anyhow::Result<super::merge::Outcome> {
        super::merge::merge_into(&self.db.conn, self, loser, winner, cause).await
    }
}

// ---------------------------------------------------------------------------------------------
// The implementation. Connection-generic, so a caller inside a transaction can share it.
// ---------------------------------------------------------------------------------------------

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

pub(super) async fn get<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let found = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    found.ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))
}

/// Attach a user's observation to a knowledge entity.
///
/// This is the only place a `knowledge_entity_id` is set. `project` reads the resulting membership
/// to compute the entity's columns, so nothing else about the move is recorded here.
///
/// The target entity must be live — `merged_into_id IS NULL` — and that condition is in the SQL
/// write itself, so a stale blocker cannot park an observation on a tombstone.
async fn attach<C: ConnectionTrait>(
    conn: &C,
    observation_id: i32,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    let attached = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE user_observed_entities o SET knowledge_entity_id = $2 \
               FROM knowledge_entities e \
              WHERE o.id = $1 AND e.id = $2 AND e.merged_into_id IS NULL",
            [observation_id.into(), knowledge_entity_id.into()],
        ))
        .await?;
    anyhow::ensure!(
        attached.rows_affected() == 1,
        "could not attach observation {observation_id} to knowledge entity {knowledge_entity_id}: \
         the entity is a tombstone, or one of the two does not exist"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{fixtures, test_db};

    /// A stale blocker can hand back an entity another writer has since merged: the write refuses.
    #[tokio::test]
    async fn an_observation_is_not_attached_to_a_tombstone() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let store = Knowledge::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train").await;
        store
            .merge(loser, winner, crate::knowledge::merge::Cause::Repair)
            .await
            .expect("the two entities are mergeable");

        let user = fixtures::user(&db, "attach").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        let observation = fixtures::entity(&db, user, capture, illumination, 0, "Train").await;

        let error = store
            .attach_observation(observation, loser)
            .await
            .expect_err("the loser is a tombstone");
        assert!(error.to_string().contains("tombstone"), "{error}");

        let row = model::user_observed_entity::Entity::find_by_id(observation)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.knowledge_entity_id, None,
            "the observation was left alone"
        );

        store
            .attach_observation(observation, winner)
            .await
            .expect("a live entity still takes it");
    }
}
