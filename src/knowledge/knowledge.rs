//! The knowledge graph's write side.
//!
//! One type, one owned handle: every write to `knowledge_entities` and every move of an observation
//! onto one goes through [`Knowledge`]. The connection-generic functions at the bottom are the
//! implementation, kept `pub(super)` so `merge` can run them inside its transaction; nothing outside
//! this module can reach a connection at all.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use crate::database::DbHandle;
use crate::model;

use super::evidence::Evidence;
use super::heuristic::{HeuristicLinker, NameBlocker};
use super::link::LinkStats;

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

/// The knowledge graph, as seen by anyone who writes to it.
#[derive(Clone)]
pub struct Knowledge {
    db: DbHandle,
}

impl Knowledge {
    pub fn new(db: &DbHandle) -> Self {
        Self { db: db.clone() }
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
    pub async fn discard(&self, knowledge_entity_id: i32) -> anyhow::Result<()> {
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
        super::link::link_illumination(
            self,
            &mut blocker,
            &evidence,
            &HeuristicLinker,
            illumination,
        )
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
    model::knowledge_entity::Entity::delete_by_id(knowledge_entity_id)
        .exec(conn)
        .await?;
    Ok(())
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
        Knowledge::new(db)
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
