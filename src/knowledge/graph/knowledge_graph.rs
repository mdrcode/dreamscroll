//! The knowledge graph's write side: [`KnowledgeGraph`], and the primitives every write shares.
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`KnowledgeGraph`], and the handle itself never leaves this module tree — the
//! primitives here are private, and the collaborators (`decider`, `blocker`, `evidence`, `verdicts`)
//! are `pub(super)` because only the passes beside the facade read them. Identity and claim writes are
//! methods ([`KnowledgeGraph::identity_for_link`], [`KnowledgeGraph::upsert_identity`],
//! [`KnowledgeGraph::assert`]), so `helper` and the resolver hold a graph, not a store. There is no
//! connection getter: `link_illumination` and `proximity` take the pool as an argument, and the two
//! stores hold their own. Outside `graph`, a caller sees [`KnowledgeGraph`]'s methods and nothing
//! else.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, QueryOrder, Set, Statement};

use crate::database::DbHandle;
use crate::knowledge::blocker::Blocker;
use crate::knowledge::evidence::{ClaimDraft, DraftIdentity, Evidence, PlatformLink, Stored};
use crate::knowledge::linker::Linker;
use super::replay::ReplayReport;
use crate::knowledge::verdicts::{Cause, VerdictStore};
use crate::model;


/// The knowledge graph, as seen by anyone who writes to it.
///
/// Most callers are a store: they create, attach, project and merge, and never decide anything. Those
/// build [`KnowledgeGraph::new`]. Linking additionally needs a decider and a blocker, both injected by
/// [`KnowledgeGraph::with_linking`].
///
/// The store holds no policy: a [`Linker`] is a procedure over the `Scoring` it was built with, and a
/// pass that measures — [`proximity`](crate::knowledge::proximity) — is handed the policy as an argument.
///
/// The blocker is built once and kept for the run's links. It queries `knowledge_blocking_index`,
/// so every writer is visible to it and nothing has to be told about a projection.
#[derive(Clone)]
pub struct KnowledgeGraph {
    db: DbHandle,
    pub(super) blocker: Option<Arc<dyn Blocker>>,
    pub(super) decider: Option<Arc<dyn Linker>>,
    pub(super) evidence: Evidence,
    pub(super) verdicts: VerdictStore,
}

impl KnowledgeGraph {
    pub fn new(db: &DbHandle) -> Self {
        Self {
            db: db.clone(),
            blocker: None,
            decider: None,
            evidence: Evidence::from_handle(db),
            verdicts: VerdictStore::from_handle(db),
        }
    }

    pub fn with_linking(
        db: &DbHandle,
        blocker: Box<dyn Blocker>,
        decider: Arc<dyn Linker>,
    ) -> Self {
        Self {
            decider: Some(decider),
            blocker: Some(blocker.into()),
            ..Self::new(db)
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

    /// The identity a capture's platform link asserts, created on first sight.
    ///
    /// A capture may create an identity, never enrich one: it knows a handle exists, not what the
    /// thing is. Idempotent, so a re-link is free.
    pub async fn identity_for_link(&self, link: &PlatformLink) -> anyhow::Result<i32> {
        self.evidence.identity_for_link(link).await
    }

    /// Record an authority's answer for an identity, filling gaps in an existing row.
    pub async fn upsert_identity(&self, draft: &DraftIdentity) -> anyhow::Result<i32> {
        self.evidence.upsert_identity(draft).await
    }

    /// Write one claim edge, unless a standing veto names it. The one writer both callers share.
    pub async fn assert(
        &self,
        draft: ClaimDraft<'_>,
        vetoes: &HashSet<(i32, String, String)>,
    ) -> anyhow::Result<Stored> {
        self.evidence.assert(draft, vetoes).await
    }

    /// Apply the recorded decisions to a rebuilt graph. See [`crate::knowledge::verdicts`].
    pub async fn replay_verdicts(&self) -> anyhow::Result<ReplayReport> {
        super::replay::replay(self).await
    }

    /// Record that an entity is *not* an identity; the next replay withdraws its claim.
    pub async fn veto(
        &self,
        entity_id: i32,
        scheme: &str,
        external_id: &str,
        note: Option<&str>,
    ) -> anyhow::Result<bool> {
        let generation = current_generation(&self.db.conn).await?;
        self.verdicts
            .veto(generation, entity_id, scheme, external_id, note)
            .await
    }

    /// Lift a veto. The next resolve may claim the identity again.
    pub async fn unveto(&self, verdict_id: i32) -> anyhow::Result<bool> {
        self.verdicts.unveto(verdict_id).await
    }

    /// The `(entity, scheme, external_id)` pairings a live veto forbids, for admission.
    pub(crate) async fn vetoed(&self) -> anyhow::Result<HashSet<(i32, String, String)>> {
        self.verdicts.vetoed().await
    }

    /// Point a user's observation to the KnowledgeGraph entity it belongs to.
    pub async fn attach_observation(
        &self,
        observation_id: i32,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<()> {
        attach(&self.db.conn, observation_id, knowledge_entity_id).await
    }

    /// Link a batch of observations into entities: decide, attach, project.
    ///
    /// Nothing here knows about identities or claims.
    /// [`crate::knowledge::link_illumination`] is the caller that batches by illumination
    /// and asserts what each capture showed.
    pub async fn link(
        &self,
        observations: &[model::user_observed_entity::Model],
    ) -> anyhow::Result<(crate::knowledge::LinkReport, std::collections::HashMap<i32, i32>)> {
        super::link::link(self, observations).await
    }

    /// Recompute an entity's projected columns.
    ///
    /// Every change to those columns funnels through here, so the graph has one definition of what
    /// they are. Implemented in `super::project`.
    pub async fn update_projection(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        super::project::project(&self.db.conn, knowledge_entity_id).await
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
        cause: Cause,
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
        status: Set(model::knowledge_entity::Status::Provisional),
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

pub(crate) async fn get<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let found = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    found.ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))
}

/// The generation the graph is being built in. One is implicit on a database that never reset.
///
/// The log stamps it on every decision; `reset` records what a generation held. A primitive rather
/// than a method because the writers that need it are free functions.
pub(crate) async fn current_generation<C: ConnectionTrait>(conn: &C) -> anyhow::Result<i32> {
    Ok(model::knowledge_generations::Entity::find()
        .order_by_desc(model::knowledge_generations::Column::Id)
        .one(conn)
        .await?
        .map_or(1, |row| row.id))
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

    use crate::test_support::{db_or_skip, fixtures};

    /// A stale blocker can hand back an entity another writer has since merged: the write refuses.
    #[tokio::test]
    async fn an_observation_is_not_attached_to_a_tombstone() {
        db_or_skip!(db);
        let store = KnowledgeGraph::new(&db);
        let winner = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        let loser = fixtures::knowledge_entity_with_member(&db, "Train").await;
        store
            .merge(loser, winner, Cause::Repair)
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
