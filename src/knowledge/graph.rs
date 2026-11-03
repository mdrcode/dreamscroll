//! The knowledge graph's write side: [`Knowledge`].
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`Knowledge`]. The connection-generic functions at the bottom are the
//! implementation, kept `pub(super)` so `merge` can run them inside its transaction; nothing outside
//! this module can reach a connection at all.

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};

use crate::database::DbHandle;
use crate::model;

use super::blocker::Blocker;
use super::evidence::{ClaimOutcome, Evidence};
use super::heuristic::NameBlocker;
use super::link_decider::{self, LinkDecider};

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

/// The knowledge graph, as seen by anyone who writes to it.
///
/// The linker is a constructor argument, not a name in here: which linker decides is a policy the
/// composition root chooses, and a second one (or a stub in a test) must not need an edit to the
/// executor.
#[derive(Clone)]
pub struct Knowledge {
    db: DbHandle,
    decider: Arc<dyn LinkDecider>,
}

impl Knowledge {
    pub fn new(db: &DbHandle, decider: Arc<dyn LinkDecider>) -> Self {
        Self {
            db: db.clone(),
            decider,
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
        discard_empty(&self.db.conn, knowledge_entity_id).await
    }

    pub async fn update_projection(
        &self,
        knowledge_entity_id: i32,
    ) -> anyhow::Result<model::knowledge_entity::Model> {
        project(&self.db.conn, knowledge_entity_id).await
    }

    /// Link one reading into the graph: cluster its observations, record their claims, and project
    /// every entity it touched.
    ///
    /// The blocker is rebuilt per call — it is an in-memory index of every entity, so a long-lived
    /// one would go stale the moment `resolve` or a merge reshapes the graph. Callers that link many
    /// illuminations in a row (`link`, `link_all`) build one blocker and use `link::link_illumination`
    /// directly.
    pub async fn link_illumination(
        &self,
        illumination: &model::illumination::ModelEx,
    ) -> anyhow::Result<LinkStats> {
        let mut blocker = NameBlocker::new(&self.db).await?;
        let evidence = Evidence::from_handle(&self.db);
        self.link_illumination_with(illumination, &mut blocker, &evidence)
            .await
    }

    /// Link one illumination by id, refusing one that belongs to somebody else.
    ///
    /// The owner is part of the read rather than a check after it: an illumination id names exactly
    /// one user's reading, and a task carrying the wrong owner should find nothing.
    pub async fn link_illumination_by_id(
        &self,
        illumination_id: i32,
        entity_owner_id: i32,
    ) -> anyhow::Result<LinkStats> {
        let illumination = model::illumination::Entity::load()
            .filter(model::illumination::Column::Id.eq(illumination_id))
            .filter(model::illumination::Column::UserId.eq(entity_owner_id))
            .with(model::user_observed_entity::Entity)
            .one(&self.db.conn)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "illumination {illumination_id} not found for user {entity_owner_id}"
                )
            })?;

        self.link_illumination(&illumination).await
    }
}

// ---------------------------------------------------------------------------------------------
// The implementation. Connection-generic, so a caller inside a transaction can share it.
// ---------------------------------------------------------------------------------------------

pub(crate) fn surfaces(
    observation: &model::user_observed_entity::Model,
) -> impl Iterator<Item = &str> {
    [
        Some(observation.name.as_str()),
        observation.platform_handle.as_deref(),
        observation.platform_display_name.as_deref(),
    ]
    .into_iter()
    .flatten()
}

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

/// Recompute the columns that are a function of the members and of the evidence.
///
/// Nothing else writes `name`, `alt_names`, `description`, `alt_descriptions`, `search_query`,
/// `alt_search_queries`, `platform`, `platform_handle`, `platform_display_name`, `status` or
/// `confidence`.
///
/// The entity **wears the values of its origin** — the member it was created from — and every other
/// member contributes alternatives. That is what makes a merge safe: the winner keeps its own
/// surface and text while the loser's arrive as `alt_*`, without anyone having to guess which of
/// them the current columns came from.
pub(super) async fn project<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    use model::knowledge_evidence as evidence;

    let row = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("knowledge entity {knowledge_entity_id} does not exist"))?;

    // Ordered by member id, so the alternatives come out in the order the members joined.
    let members = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(conn)
        .await?;

    // An entity that predates the column, or whose origin is gone, falls back to its first member.
    let origin = row
        .origin_observation_id
        .and_then(|id| members.iter().find(|member| member.id == id))
        .or_else(|| members.first());

    let surfaces = gather(
        &members,
        |member| surfaces(member).map(str::to_string).collect(),
        super::normalize::key,
    );
    let types = gather(
        &members,
        |member| vec![member.entity_type.clone()],
        str::to_string,
    );
    let descriptions = gather(
        &members,
        |member| member.description.clone().into_iter().collect(),
        str::to_string,
    );
    let queries = gather(
        &members,
        |member| member.search_query.clone().into_iter().collect(),
        str::to_string,
    );

    let (name, alt_names) = select(origin.map(|member| member.name.trim()), &surfaces);
    let (entity_type, alt_types) = select(origin.map(|member| member.entity_type.trim()), &types);

    // When no member offers text, keep what is there: an authority-sourced description must not be
    // wiped by a member list that says nothing.
    let (description, alt_descriptions) = match select(
        origin.and_then(|member| member.description.as_deref()),
        &descriptions,
    ) {
        (Some(text), alt) => (Some(text), alt),
        (None, _) => (row.description.clone(), row.alt_descriptions.clone()),
    };
    let (search_query, alt_search_queries) = match select(
        origin.and_then(|member| member.search_query.as_deref()),
        &queries,
    ) {
        (Some(text), alt) => (Some(text), alt),
        (None, _) => (row.search_query.clone(), row.alt_search_queries.clone()),
    };

    // The presence the resolver asks by: the origin's if it showed one, else the first member's.
    let presence = origin
        .filter(|member| member.platform.is_some())
        .or_else(|| members.iter().find(|member| member.platform.is_some()));
    let (platform, platform_handle, platform_display_name) = match presence {
        Some(member) => (
            member.platform.clone(),
            member.platform_handle.clone(),
            member.platform_display_name.clone(),
        ),
        None => (
            row.platform.clone(),
            row.platform_handle.clone(),
            row.platform_display_name.clone(),
        ),
    };

    let evidence_rows = evidence::Entity::find()
        .filter(evidence::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .filter(evidence::Column::Kind.is_in(["claim", "collision"]))
        .filter(evidence::Column::RetractedAt.is_null())
        .all(conn)
        .await?;
    let contested = evidence_rows.iter().any(|row| row.kind == "collision");
    let claims: Vec<_> = evidence_rows
        .iter()
        .filter(|row| row.kind == "claim")
        .collect();

    let status = if contested {
        "contested"
    } else if claims.is_empty() {
        STATUS_PROVISIONAL
    } else {
        STATUS_IDENTIFIED
    };

    // Corroboration counts independent *inputs*, not scheme labels. A `p2002` claim is derived from
    // the handle — the resolver asked Wikidata which item owns that account — so counting it beside
    // the handle's own row treated one chain as two and gave every handle-resolved entity maximum
    // confidence.
    let from_text = claims
        .iter()
        .any(|claim| claim.source == "wikidata" && claim.method.as_deref() != Some("p2002"));
    let from_handle = claims.iter().any(|claim| claim.source == "platform");
    let inputs = usize::from(from_text) + usize::from(from_handle);
    let best = claims
        .iter()
        .map(|claim| claim.confidence.unwrap_or(0.0))
        .fold(0.0f32, f32::max);
    let confidence = if claims.is_empty() {
        None
    } else {
        Some((best + inputs.saturating_sub(1) as f32 * 0.1).min(1.0))
    };

    model::knowledge_entity::Entity::update(model::knowledge_entity::ActiveModel {
        id: Set(knowledge_entity_id),
        name: Set(name.unwrap_or_else(|| row.name.clone())),
        alt_names: Set(alt_names),
        entity_type: Set(entity_type.unwrap_or_else(|| row.entity_type.clone())),
        alt_types: Set(alt_types),
        description: Set(description),
        alt_descriptions: Set(alt_descriptions),
        search_query: Set(search_query),
        alt_search_queries: Set(alt_search_queries),
        platform: Set(platform),
        platform_handle: Set(platform_handle),
        platform_display_name: Set(platform_display_name),
        status: Set(status.to_string()),
        confidence: Set(confidence),
        ..Default::default()
    })
    .exec(conn)
    .await
    .map_err(Into::into)
}

/// Every distinct, non-empty value the members offer, in member order.
fn gather(
    members: &[model::user_observed_entity::Model],
    values: impl Fn(&model::user_observed_entity::Model) -> Vec<String>,
    key: impl Fn(&str) -> String,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut distinct = Vec::new();
    for member in members {
        for value in values(member) {
            let text = value.trim();
            if !text.is_empty() && seen.insert(key(text)) {
                distinct.push(text.to_string());
            }
        }
    }
    distinct
}

/// Keep `preferred` while the members still offer it; otherwise take the first. Everything else
/// becomes the alternative list.
fn select(preferred: Option<&str>, candidates: &[String]) -> (Option<String>, Vec<String>) {
    let selected = preferred
        .filter(|text| candidates.iter().any(|candidate| candidate == text))
        .map(str::to_string)
        .or_else(|| candidates.first().cloned());
    let alternatives = candidates
        .iter()
        .filter(|candidate| Some(candidate.as_str()) != selected.as_deref())
        .cloned()
        .collect();
    (selected, alternatives)
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

// ---------------------------------------------------------------------------------------------
// Linking: the one write path that decides before it writes.
// ---------------------------------------------------------------------------------------------

/// What one illumination's decisions actually did.
///
/// Deliberately not a report: it does not count what the caller already knows — how many observations
/// were considered, which illuminations were superseded. It says only what the executor wrote.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub attached: usize,
    /// The part of `attached` where a claim named the entity rather than a score.
    pub attached_by_claim: usize,
    pub created: usize,
    /// Created, then dropped: another entity turned out to hold the claim after all.
    pub folded: usize,
    /// The same-illumination veto firing.
    pub vetoed: u32,
}

impl LinkStats {
    /// Fold one decision and what came of it into the totals.
    ///
    /// Both halves live here because they are one question: what did this reading do, and why.
    fn record(&mut self, decision: &link_decider::Decision, applied: Applied) {
        match &decision.action {
            link_decider::Action::Attach { by_claim: true, .. } => self.attached_by_claim += 1,
            link_decider::Action::Create { vetoed: true, .. } => self.vetoed += 1,
            _ => {}
        }
        match applied.outcome {
            Outcome::Attached => self.attached += 1,
            Outcome::Created => self.created += 1,
            Outcome::Folded => self.folded += 1,
        }
    }
}

/// What applying one decision did.
struct Applied {
    /// The entity the observation ended up on — the one to update the projection for.
    entity: i32,
    outcome: Outcome,
}

enum Outcome {
    /// It joined the entity the decision named.
    Attached,
    /// A new entity was created for it.
    Created,
    /// A new entity was created and then discarded, because another one already held the claim.
    Folded,
}

/// Apply a linking decision against the knowledge_entities table.
impl Knowledge {
    /// Link one illumination with a caller's blocker and evidence: they are the expensive, shareable
    /// things, so a caller linking several illuminations builds them once. The linking policy is the
    /// one this store was built with. [`Self::link_illumination`] builds the collaborators too.
    pub async fn link_illumination_with(
        &self,
        illumination: &model::illumination::ModelEx,
        blocker: &mut dyn Blocker,
        evidence: &Evidence,
    ) -> anyhow::Result<LinkStats> {
        let decisions = self
            .decider
            .decide_link(blocker, evidence, illumination)
            .await?;

        let mut stats = LinkStats::default();
        for decision in &decisions {
            let applied = self.apply_decision(evidence, decision).await?;
            // Projection is a function of the members, so it runs after the writes that changed them,
            // and it returns the row it wrote — which is exactly what the blocker needs to re-index.
            // An entity two observations landed on is projected twice; a wasted read is cheaper than
            // remembering which ones were already done.
            let entity = self.update_projection(applied.entity).await?;
            blocker.changed(&entity);
            stats.record(decision, applied);
        }

        Ok(stats)
    }

    /// Apply one decision against the graph.
    async fn apply_decision(
        &self,
        evidence: &Evidence,
        decision: &link_decider::Decision,
    ) -> anyhow::Result<Applied> {
        let observation = &decision.observation;
        if observation.knowledge_entity_id.is_some() {
            anyhow::bail!(
                "the linker decided about observation {}, which is already linked",
                observation.id
            );
        }

        // The entity this observation ends up on, resolved once: the one the decision named, or a new
        // one it asked for.
        let mut target_entity = match &decision.action {
            link_decider::Action::Attach { entity, .. } => self.get(*entity).await?,
            link_decider::Action::Create { .. } => self.create(observation).await?,
        };
        let created = target_entity.id;
        self.attach_observation(observation.id, target_entity.id)
            .await?;

        // The observation may carry its own claim — a handle we inferred from the capture. If another
        // entity already holds it, the entity just created owns nothing: discard it and point the
        // observation at the one that does. Usually this is two observations of one illumination that
        // both asked for a new entity carrying the same claim; occasionally it is another worker.
        let mut folded = false;
        if let Some(claim) = &decision.claim
            && let ClaimOutcome::HeldBy(owner) = evidence
                .record(target_entity.id, claim, observation)
                .await?
        {
            if matches!(decision.action, link_decider::Action::Attach { .. }) {
                anyhow::bail!(
                    "entities {} and {owner} both hold {}:{}; merging is not implemented yet \
                 (plan/knowledge-canonical-entities.md §Repair)",
                    target_entity.id,
                    claim.scheme,
                    claim.id
                );
            }
            // Re-point before discarding: the observation still references the entity being deleted,
            // and that foreign key is `NO ACTION`.
            target_entity = self.get(owner).await?;
            self.attach_observation(observation.id, owner).await?;
            self.discard_empty(created).await?;
            folded = true;
        }

        let outcome = if folded {
            Outcome::Folded
        } else if matches!(decision.action, link_decider::Action::Attach { .. }) {
            Outcome::Attached
        } else {
            Outcome::Created
        };
        Ok(Applied {
            entity: target_entity.id,
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        knowledge_entity as entity, knowledge_evidence as evidence,
        user_observed_entity as observation,
    };
    use crate::test_support::{fixtures, test_db};

    fn store(db: &DbHandle) -> Knowledge {
        Knowledge::new(db, Arc::new(crate::knowledge::heuristic::HeuristicLinker))
    }

    /// One entity with one observation attached, ready to update.
    async fn seed(db: &DbHandle, name: &str) -> i32 {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id =
            fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
        let knowledge_entity_id = entity::Entity::insert(entity::ActiveModel {
            name: Set(name.to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(STATUS_PROVISIONAL.to_string()),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;
        observation::Entity::update_many()
            .col_expr(
                observation::Column::KnowledgeEntityId,
                Expr::value(Some(knowledge_entity_id)),
            )
            .filter(observation::Column::Id.eq(observation_id))
            .exec(&db.conn)
            .await
            .unwrap();
        knowledge_entity_id
    }

    async fn claim(db: &DbHandle, id: i32, kind: &str, scheme: &str, value: &str) {
        evidence::Entity::insert(evidence::ActiveModel {
            knowledge_entity_id: Set(id),
            kind: Set(kind.to_string()),
            value: Set(serde_json::json!({ "scheme": scheme, "id": value })),
            source: Set("test".to_string()),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();
    }

    async fn load(db: &DbHandle, id: i32) -> entity::Model {
        entity::Entity::find_by_id(id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn the_selected_surface_is_not_one_of_its_own_alternatives() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.name, "Train Dreams");
        assert!(
            !entity.alt_names.iter().any(|name| name == "Train Dreams"),
            "alt_names is the complement of the selected name: {:?}",
            entity.alt_names
        );
        assert_eq!(entity.status, STATUS_PROVISIONAL);
        assert_eq!(entity.confidence, None);
    }

    /// Another member of the same entity, carrying its own hint.
    async fn attach_member(db: &DbHandle, id: i32, name: &str, query: Option<&str>) -> i32 {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id =
            fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
        observation::Entity::update_many()
            .col_expr(
                observation::Column::SearchQuery,
                Expr::value(query.map(str::to_string)),
            )
            .col_expr(
                observation::Column::KnowledgeEntityId,
                Expr::value(Some(id)),
            )
            .filter(observation::Column::Id.eq(observation_id))
            .exec(&db.conn)
            .await
            .unwrap();
        observation_id
    }

    /// A member that carries a platform presence.
    async fn attach_link(db: &DbHandle, id: i32, name: &str, platform: &str, handle: &str) {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id =
            fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
        observation::Entity::update_many()
            .col_expr(
                observation::Column::Platform,
                Expr::value(Some(platform.to_string())),
            )
            .col_expr(
                observation::Column::PlatformHandle,
                Expr::value(Some(handle.to_string())),
            )
            .col_expr(
                observation::Column::KnowledgeEntityId,
                Expr::value(Some(id)),
            )
            .filter(observation::Column::Id.eq(observation_id))
            .exec(&db.conn)
            .await
            .unwrap();
    }

    /// The platform presence, projected from whichever member carried one.
    #[tokio::test]
    async fn the_platform_presence_is_projected_from_a_member() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Tsoding").await;
        attach_link(&db, id, "Tsoding", "x_twitter", "tsoding").await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.platform.as_deref(), Some("x_twitter"));
        assert_eq!(entity.platform_handle.as_deref(), Some("tsoding"));
        assert_eq!(entity.platform_display_name, None);
    }

    /// The hint is a property of the entity, projected like the description: one selected, the rest
    /// kept, in member order so the answer never depends on arrival order.
    #[tokio::test]
    async fn the_hint_is_projected_with_its_alternatives() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        attach_member(&db, id, "Train Dreams", Some("Train Dreams novella")).await;
        attach_member(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson")).await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.search_query.as_deref(), Some("Train Dreams novella"));
        assert_eq!(
            entity.alt_search_queries,
            vec!["Train Dreams Denis Johnson"],
            "the selected hint is never one of its own alternatives"
        );
    }

    /// The entity wears its origin's values, whatever the row said before — and the origin is the
    /// member it was created from, not simply the first one.
    #[tokio::test]
    async fn the_origin_member_decides_which_value_is_worn() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        attach_member(&db, id, "Train Dreams", Some("Train Dreams novella")).await;
        let second =
            attach_member(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson")).await;

        // The row is told a different member is its origin.
        entity::Entity::update(entity::ActiveModel {
            id: Set(id),
            origin_observation_id: Set(Some(second)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(
            entity.search_query.as_deref(),
            Some("Train Dreams Denis Johnson"),
            "the origin decides"
        );
    }

    /// A decider that decides nothing. Exists to prove the policy is the caller's: if this store
    /// still named `HeuristicLinker` somewhere inside, this test could not pass.
    struct NoDecisions;

    #[async_trait::async_trait]
    impl LinkDecider for NoDecisions {
        async fn decide_link(
            &self,
            _blocker: &dyn Blocker,
            _evidence: &Evidence,
            _illumination: &model::illumination::ModelEx,
        ) -> anyhow::Result<Vec<link_decider::Decision>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn the_linking_policy_is_the_one_it_was_built_with() {
        let Some(test_db) = test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user = fixtures::user(&db, "policy").await;
        let capture = fixtures::capture(&db, user).await;
        let illumination = fixtures::illumination(&db, user, capture).await;
        fixtures::entity(&db, user, capture, illumination, 0, "Train Dreams").await;

        let inert = Knowledge::new(&db, Arc::new(NoDecisions));
        let stats = inert
            .link_illumination_by_id(illumination, user)
            .await
            .unwrap();
        assert_eq!(
            stats,
            LinkStats::default(),
            "nothing decided, nothing written"
        );
        assert_eq!(
            entity::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "an inert policy must not create entities"
        );

        let linking = Knowledge::new(&db, Arc::new(crate::knowledge::heuristic::HeuristicLinker));
        let stats = linking
            .link_illumination_by_id(illumination, user)
            .await
            .unwrap();
        assert_eq!(stats.created, 1, "the linker it was built with decides");
    }

    /// The name is the contract: an entity that still owns observations is refused, not deleted.
    /// (The foreign key would fail it too — but on a call that was never meant to reach the database.)
    #[tokio::test]
    async fn an_entity_with_members_is_not_discarded() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;

        let error = store(&db).discard_empty(id).await.unwrap_err();
        assert!(error.to_string().contains("still owns"), "{error}");

        assert_eq!(load(&db, id).await.id, id, "the row must survive");
    }

    #[tokio::test]
    async fn a_live_claim_makes_it_identified() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "claim", "wikidata", "Q104919").await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.status, STATUS_IDENTIFIED);
        assert!(entity.confidence.is_some(), "a claim carries a confidence");
    }

    #[tokio::test]
    async fn a_collision_makes_it_contested() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "collision", "wikidata", "Q104919").await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.status, "contested");
        assert_eq!(entity.confidence, None, "a collision is not an identity");
    }

    #[tokio::test]
    async fn a_retracted_claim_does_not_identify() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "claim", "wikidata", "Q104919").await;
        evidence::Entity::update_many()
            .col_expr(
                evidence::Column::RetractedAt,
                Expr::value(chrono::Utc::now()),
            )
            .filter(evidence::Column::KnowledgeEntityId.eq(id))
            .exec(&db.conn)
            .await
            .unwrap();

        store(&db).update_projection(id).await.unwrap();

        assert_eq!(load(&db, id).await.status, STATUS_PROVISIONAL);
    }
}

#[cfg(test)]
mod linking {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};

    use super::*;
    use crate::database;
    use crate::knowledge::{STATUS_IDENTIFIED, STATUS_PROVISIONAL};
    use crate::model::{
        knowledge_entity as entity, knowledge_evidence as evidence,
        user_observed_entity as observation,
    };
    use crate::test_support::fixtures;

    struct Capture {
        user_id: i32,
        capture_id: i32,
        illumination_id: i32,
    }

    async fn capture(db: &database::DbHandle, tag: &str) -> Capture {
        let user_id = fixtures::user(db, tag).await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        Capture {
            user_id,
            capture_id,
            illumination_id,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn observe(
        db: &database::DbHandle,
        capture: &Capture,
        position: i32,
        name: &str,
        description: Option<&str>,
        entity_type: &str,
        platform: Option<&str>,
        handle: Option<&str>,
    ) -> i32 {
        observation::Entity::insert(observation::ActiveModel {
            user_id: Set(capture.user_id),
            capture_id: Set(capture.capture_id),
            illumination_id: Set(capture.illumination_id),
            position: Set(position),
            name: Set(name.to_string()),
            entity_type: Set(entity_type.to_string()),
            description: Set(description.map(str::to_string)),
            platform: Set(platform.map(str::to_string)),
            platform_handle: Set(handle.map(str::to_string)),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .expect("observation should insert")
        .last_insert_id as i32
    }

    async fn entity_count(db: &database::DbHandle) -> u64 {
        entity::Entity::find().count(&db.conn).await.unwrap()
    }

    #[tokio::test]
    async fn one_capture_never_folds_two_observations_together() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_same_capture").await;
        observe(
            &db,
            &same,
            0,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;
        observe(
            &db,
            &same,
            1,
            "Dune",
            Some("a 1965 novel by Frank Herbert"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(entity_count(&db).await, 2);
    }

    /// Two observations of one illumination carrying the same, previously unseen handle. Decisions
    /// are made before any writes, so neither can see the other's claim: both are told to create.
    /// The second discovers the first already recorded it and folds — re-pointing the observation
    /// first, because the row it created cannot be deleted while an observation still references it.
    #[tokio::test]
    async fn two_observations_sharing_a_new_handle_fold_into_one_entity() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let same = capture(&db, "link_fold").await;
        observe(
            &db,
            &same,
            0,
            "Alexey Kutepov",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        observe(
            &db,
            &same,
            1,
            "Tsoding",
            None,
            "social_media_account",
            Some("x"),
            Some("Tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("the second must fold, not trip the foreign key");

        assert_eq!(entity_count(&db).await, 1, "one referent, not two");
        let linked: Vec<Option<i32>> = observation::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.knowledge_entity_id)
            .collect();
        assert_eq!(linked.len(), 2);
        assert!(linked.iter().all(|id| *id == linked[0] && id.is_some()));
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "one claim, on the entity that survived"
        );
    }

    #[tokio::test]
    async fn a_second_run_does_nothing() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_idempotent").await;
        observe(
            &db,
            &first,
            0,
            "Train Dreams",
            Some("a novella"),
            "book",
            None,
            None,
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");
        crate::test_support::link::link_all(&db)
            .await
            .expect("second build should succeed");

        assert_eq!(
            entity_count(&db).await,
            1,
            "one entity, and a second run adds none"
        );
        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "the second run had nothing left to link");
    }

    /// Every illumination is linked. An illumination can be wrong and the correct illumination is often the
    /// older one, so the graph keeps them all — currency is decided at the view boundary
    /// (`api::user::get_entity`), not here.
    #[tokio::test]
    async fn a_superseded_illumination_is_linked_too() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let user_id = fixtures::user(&db, "link_superseded").await;
        let capture_id = fixtures::capture(&db, user_id).await;
        let old = fixtures::illumination(&db, user_id, capture_id).await;
        let live = fixtures::illumination(&db, user_id, capture_id).await;

        for (illumination_id, name) in [(old, "Old Run"), (live, "Train Dreams")] {
            observation::Entity::insert(observation::ActiveModel {
                user_id: Set(user_id),
                capture_id: Set(capture_id),
                illumination_id: Set(illumination_id),
                position: Set(0),
                name: Set(name.to_string()),
                entity_type: Set("book".to_string()),
                ..Default::default()
            })
            .exec(&db.conn)
            .await
            .unwrap();
        }

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let unlinked = observation::Entity::find()
            .filter(observation::Column::KnowledgeEntityId.is_null())
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(unlinked, 0, "history is linked, not discarded");
    }

    /// `project` keeps the selected name out of `alt_names`, so an observation is only findable on a
    /// later run if its own name is indexed as well. Without that, every capture that repeated a
    /// name created a duplicate — the local corpus had ten such pairs.
    #[tokio::test]
    async fn a_shared_handle_links_across_captures_and_names() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_persona_first").await;
        let second = capture(&db, "link_persona_second").await;
        observe(
            &db,
            &first,
            0,
            "Alexey Kutepov",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        observe(
            &db,
            &second,
            0,
            "Tsoding",
            None,
            "social_media_account",
            Some("x"),
            Some("Tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        assert_eq!(entity_count(&db).await, 1, "the claim folded the duplicate");
        assert_eq!(evidence::Entity::find().count(&db.conn).await.unwrap(), 1);
    }

    /// The concurrency guard, tested directly: a second entity asking for an identifier that is
    /// already claimed must be told who owns it, not silently ignored.
    #[tokio::test]
    async fn an_identifier_claimed_elsewhere_reports_the_owner() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_claim").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;
        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let owner_core = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        let duplicate = entity::Entity::insert(entity::ActiveModel {
            name: Set("Tsoding".to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set(STATUS_PROVISIONAL.to_string()),
            alt_names: Set(vec!["tsoding".to_string()]),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;

        let observation = observation::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let claim = crate::knowledge::evidence::Claim::of(&observation).expect("a handle");
        let outcome = Evidence::from_handle(&db)
            .record(duplicate, &claim, &observation)
            .await
            .expect("claim write should succeed");

        assert_eq!(outcome, ClaimOutcome::HeldBy(owner_core.id));
        assert_eq!(
            evidence::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "the duplicate must not create a second identity row"
        );
    }

    #[tokio::test]
    async fn a_handle_identified_core_is_not_provisional() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let first = capture(&db, "link_status").await;
        observe(
            &db,
            &first,
            0,
            "Tsoding",
            None,
            "real_person",
            Some("x"),
            Some("tsoding"),
        )
        .await;

        crate::test_support::link::link_all(&db)
            .await
            .expect("build should succeed");

        let entity = entity::Entity::find().one(&db.conn).await.unwrap().unwrap();
        assert_eq!(entity.status, STATUS_IDENTIFIED);
    }
}
