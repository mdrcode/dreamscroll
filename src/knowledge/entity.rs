//! The write side of an entity: create one, attach a member to it, absorb what that member knows,
//! and recompute its derived columns.
//!
//! `model::knowledge_entity` says what an entity *is*; this says who is allowed to change one.
//!
//! Three rules, and they are why these functions share a file:
//!
//! - [`attach`] is the only thing that writes `user_observed_entities.knowledge_entity_id`.
//! - [`absorb_observation`] accumulates a member's surfaces and type into the row it joins — an
//!   incremental step, not the truth.
//! - [`project`] recomputes every derived column from the members and the evidence, and is
//!   authoritative: nothing else writes `name`, `description`, `status`, `confidence`,
//!   `search_query`, `alt_search_queries` or the `platform_*` triple.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};

use crate::model;

use super::normalize;

pub const STATUS_PROVISIONAL: &str = "provisional";
pub const STATUS_IDENTIFIED: &str = "identified";

/// Add an observed type. Returns whether anything changed: an entity that has accepted `brand`
/// must accept it for the rest of the run.
fn absorb_type(entity: &mut model::knowledge_entity::Model, entity_type: &str) -> bool {
    if entity_type.is_empty() || entity.alt_types.iter().any(|t| t == entity_type) {
        return false;
    }
    entity.alt_types.push(entity_type.to_string());
    true
}

/// Add a surface. Returns whether anything changed, so a caller can skip the write.
fn absorb_surface(entity: &mut model::knowledge_entity::Model, surface: &str) -> bool {
    let key = normalize::key(surface);
    if key.is_empty() || entity.surfaces().any(|seen| normalize::key(seen) == key) {
        return false;
    }
    entity.alt_names.push(surface.trim().to_string());
    true
}

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

fn names_for(observation: &model::user_observed_entity::Model) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for surface in surfaces(observation) {
        let label = surface.trim();
        let key = normalize::key(label);
        if key.is_empty() || keys.contains(&key) {
            continue;
        }
        keys.push(key);
        labels.push(label.to_string());
    }
    labels
}

/// Point an observation at its entity.
///
/// This is the only place a `knowledge_entity_id` is set. `project` reads the resulting membership to
/// compute the entity's columns, so nothing else about the move needs recording here.
pub(crate) async fn attach(
    conn: &sea_orm::DatabaseConnection,
    entity_id: i32,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    model::user_observed_entity::Entity::update_many()
        .col_expr(
            model::user_observed_entity::Column::KnowledgeEntityId,
            Expr::value(Some(knowledge_entity_id)),
        )
        .filter(model::user_observed_entity::Column::Id.eq(entity_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Fold everything one observation contributes into the entity it was linked to: its surfaces (so a
/// later observation can find the entity by any of them) and its type. Descriptions accumulate in
/// `project`, which runs once at the end of a build rather than per observation.
/// Fold what an observation knows into its entity. Returns the updated row when anything changed,
/// so the caller can hand it to the blocker.
///
/// Read here rather than passed in: the blocker owns the corpus, so the executor keeps no parallel
/// copy of the entities and cannot hold one stale.
pub(crate) async fn absorb_observation(
    conn: &sea_orm::DatabaseConnection,
    knowledge_entity_id: i32,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<Option<model::knowledge_entity::Model>> {
    let Some(mut entity) = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?
    else {
        return Ok(None);
    };

    let mut changed = false;
    for surface in surfaces(observation) {
        changed |= absorb_surface(&mut entity, surface);
    }
    changed |= absorb_type(&mut entity, &observation.entity_type);
    if !changed {
        return Ok(None);
    }
    model::knowledge_entity::ActiveModel {
        id: Set(entity.id),
        alt_names: Set(entity.alt_names.clone()),
        alt_types: Set(entity.alt_types.clone()),
        ..Default::default()
    }
    .update(conn)
    .await?;
    Ok(Some(entity))
}

/// Record the observation's identifier against its entity. Returns the *other* entity that already
/// claims it, if any — the caller decides what to do, because dropping the claim silently would
/// leave two entities for one referent and no trace of the race.
///
/// The read-back is what makes this race-safe: `ON CONFLICT DO NOTHING` leaves whichever row was
/// committed first, and comparing its owner is cheaper and clearer than matching a unique-violation
/// error code.
pub(crate) async fn create(
    conn: &sea_orm::DatabaseConnection,
    observation: &model::user_observed_entity::Model,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let description = observation.description.clone();
    // Always provisional: a new entity has nothing proving it yet. If the capture asserted a claim,
    // it is recorded against this entity and `project` promotes it to `identified` moments later —
    // guessing the status here would be a second writer of a projected column.
    let status = STATUS_PROVISIONAL;
    let model = model::knowledge_entity::ActiveModel {
        name: Set(observation.name.trim().to_string()),
        entity_type: Set(observation.entity_type.clone()),
        alt_types: Set(Vec::new()),
        description: Set(description),
        alt_descriptions: Set(Vec::new()),
        status: Set(status.to_string()),
        confidence: Set(None),
        merged_into_id: Set(None),
        alt_names: Set(names_for(observation)),
        ..Default::default()
    };
    Ok(model::knowledge_entity::Entity::insert(model)
        .exec_with_returning(conn)
        .await?)
}



/// Recompute the columns that are a function of the members and of the evidence. Nothing else writes
/// `name`, `alt_names`, `description`, `alt_descriptions`, `search_query`, `alt_search_queries`,
/// `platform`, `platform_handle`, `platform_display_name`, `status` or `confidence`.
///
/// Deriving the surface and text here rather than at write time is what makes a merge safe: a merge
/// re-points observations and projects the winner, so the loser's names and descriptions are picked
/// up from the members that moved rather than being lost with the deleted row.
pub async fn project<C: sea_orm::ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<()> {
    use model::knowledge_evidence as evidence;

    // Ordered by member id, so the result never depends on the order members arrived in.
    let members = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(conn)
        .await?;
    let mut label_keys = HashSet::new();
    let mut labels: Vec<String> = Vec::new();
    let mut seen_descriptions = HashSet::new();
    let mut descriptions: Vec<String> = Vec::new();
    let mut seen_queries = HashSet::new();
    let mut queries: Vec<String> = Vec::new();
    let mut links: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let mut types: Vec<String> = Vec::new();
    for member in &members {
        if !types.contains(&member.entity_type) {
            types.push(member.entity_type.clone());
        }
        for surface in surfaces(member) {
            let text = surface.trim();
            if !text.is_empty() && label_keys.insert(super::normalize::key(text)) {
                labels.push(text.to_string());
            }
        }
        if let Some(text) = member
            .description
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            && seen_descriptions.insert(text.to_string())
        {
            descriptions.push(text.to_string());
        }
        if let Some(text) = member
            .search_query
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            && seen_queries.insert(text.to_string())
        {
            queries.push(text.to_string());
        }
        // The platform presence, if this member carried one. One per entity is enough for the
        // resolver, and a member has at most one.
        if let Some(platform) = member
            .platform
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            && !links.iter().any(|(seen, _, _)| seen == platform)
        {
            links.push((
                platform.to_string(),
                member.platform_handle.clone(),
                member.platform_display_name.clone(),
            ));
        }
    }

    let current = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    let (
        existing_name,
        existing_type,
        existing_description,
        existing_alt_descriptions,
        existing_query,
        existing_alt_queries,
        existing_link,
    ) = match &current {
        Some(row) => (
            row.name.clone(),
            row.entity_type.clone(),
            row.description.clone(),
            row.alt_descriptions.clone(),
            row.search_query.clone(),
            row.alt_search_queries.clone(),
            (
                row.platform.clone(),
                row.platform_handle.clone(),
                row.platform_display_name.clone(),
            ),
        ),
        None => (
            String::new(),
            String::new(),
            None,
            Vec::new(),
            None,
            Vec::new(),
            (None, None, None),
        ),
    };

    // The selected surface survives a merge as long as a member still carries it.
    let name = if labels.is_empty() || labels.contains(&existing_name) {
        existing_name
    } else {
        labels[0].clone()
    };
    let alt_labels = labels
        .iter()
        .filter(|candidate| candidate.as_str() != name)
        .cloned()
        .collect();

    let entity_type = if types.is_empty() || types.contains(&existing_type) {
        existing_type
    } else {
        types[0].clone()
    };
    let alt_types = types
        .iter()
        .filter(|candidate| *candidate != &entity_type)
        .cloned()
        .collect();

    // Same rule for the text, and the existing description is preserved when no member has any —
    // an authority-sourced description must not be wiped by a member list that says nothing.
    let (description, alt_descriptions) = if descriptions.is_empty() {
        (existing_description, existing_alt_descriptions)
    } else {
        let selected = match existing_description {
            Some(text) if descriptions.contains(&text) => Some(text),
            _ => descriptions.first().cloned(),
        };
        let alt = descriptions
            .iter()
            .filter(|text| Some(text.as_str()) != selected.as_deref())
            .cloned()
            .collect();
        (selected, alt)
    };

    // The hint is projected exactly like the description: the one already selected survives while a
    // member still carries it, so a search query is never silently swapped for another member's.
    let (search_query, alt_search_queries) = if queries.is_empty() {
        (existing_query, existing_alt_queries)
    } else {
        let selected = match existing_query {
            Some(text) if queries.contains(&text) => Some(text),
            _ => queries.first().cloned(),
        };
        let alt = queries
            .iter()
            .filter(|text| Some(text.as_str()) != selected.as_deref())
            .cloned()
            .collect();
        (selected, alt)
    };

    // The platform presence follows the same rule as everything else here: the one already chosen
    // survives while a member still carries it, so a merge does not silently swap it.
    let (platform, platform_handle, platform_display_name) = if links.is_empty() {
        existing_link
    } else {
        let (chosen_platform, chosen_handle, _) = existing_link.clone();
        let selected = chosen_platform
            .as_ref()
            .and_then(|chosen| links.iter().find(|(p, h, _)| p == chosen && h == &chosen_handle))
            .cloned()
            .unwrap_or_else(|| links[0].clone());
        (Some(selected.0), selected.1, selected.2)
    };

    let claims = evidence::Entity::find()
        .filter(evidence::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .filter(evidence::Column::Kind.eq("claim"))
        .filter(evidence::Column::RetractedAt.is_null())
        .all(conn)
        .await?;
    let contested = evidence::Entity::find()
        .filter(evidence::Column::KnowledgeEntityId.eq(knowledge_entity_id))
        .filter(evidence::Column::Kind.eq("collision"))
        .filter(evidence::Column::RetractedAt.is_null())
        .count(conn)
        .await?
        > 0;

    let status = if contested {
        "contested"
    } else if claims.is_empty() {
        "provisional"
    } else {
        "identified"
    };

    // Corroboration: each additional *independent scheme* agreeing raises confidence. Two authorities
    // asserting the same referent is a stronger claim than either alone.
    // Corroboration counts independent *inputs*, not scheme labels. A `p2002` claim is derived from
    // the handle — the resolver asked Wikidata which item owns that account — so counting it beside
    // the handle's own `platform_link` row treated one chain as two and gave every handle-resolved
    // entity the maximum confidence.
    let from_text = claims
        .iter()
        .any(|claim| claim.source == "wikidata" && claim.method.as_deref() != Some("p2002"));
    let from_handle = claims.iter().any(|claim| claim.source == "platform");
    let inputs = usize::from(from_text) + usize::from(from_handle);

    let mut best = 0.0f32;
    for claim in &claims {
        best = best.max(claim.confidence.unwrap_or(0.0));
    }
    let corroboration = inputs.saturating_sub(1) as f32 * 0.1;
    let confidence = if claims.is_empty() {
        None
    } else {
        Some((best + corroboration).min(1.0))
    };

    model::knowledge_entity::Entity::update_many()
        .col_expr(
            model::knowledge_entity::Column::Status,
            Expr::value(status),
        )
        .col_expr(
            model::knowledge_entity::Column::Confidence,
            Expr::value(confidence),
        )
        .filter(model::knowledge_entity::Column::Id.eq(knowledge_entity_id))
        .exec(conn)
        .await?;
    model::knowledge_entity::Entity::update(model::knowledge_entity::ActiveModel {
        id: Set(knowledge_entity_id),
        name: Set(name),
        alt_names: Set(alt_labels),
        entity_type: Set(entity_type),
        alt_types: Set(alt_types),
        description: Set(description),
        alt_descriptions: Set(alt_descriptions),
        search_query: Set(search_query),
        alt_search_queries: Set(alt_search_queries),
        platform: Set(platform),
        platform_handle: Set(platform_handle),
        platform_display_name: Set(platform_display_name),
        ..Default::default()
    })
    .exec(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{knowledge_entity as entity, knowledge_evidence as evidence, user_observed_entity as observation};
    use crate::test_support::{fixtures, test_db};

    /// One entity with one observation attached, ready to project.
    async fn seed(db: &crate::database::DbHandle, name: &str) -> i32 {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id =
            fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
        let knowledge_entity_id = entity::Entity::insert(entity::ActiveModel {
            name: Set(name.to_string()),
            entity_type: Set("real_person".to_string()),
            status: Set("provisional".to_string()),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32;
        observation::Entity::update_many()
            .col_expr(observation::Column::KnowledgeEntityId, Expr::value(Some(knowledge_entity_id)))
            .filter(observation::Column::Id.eq(observation_id))
            .exec(&db.conn)
            .await
            .unwrap();
        knowledge_entity_id
    }

    async fn claim(db: &crate::database::DbHandle, id: i32, kind: &str, scheme: &str, value: &str) {
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

    async fn load(db: &crate::database::DbHandle, id: i32) -> entity::Model {
        entity::Entity::find_by_id(id).one(&db.conn).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn the_selected_surface_is_not_one_of_its_own_alternatives() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.name, "Train Dreams");
        assert!(
            !entity.alt_names.iter().any(|name| name == "Train Dreams"),
            "alt_names is the complement of the selected name: {:?}",
            entity.alt_names
        );
        assert_eq!(entity.status, "provisional");
        assert_eq!(entity.confidence, None);
    }

    /// Another member of the same entity, carrying its own hint.
    async fn attach(db: &crate::database::DbHandle, id: i32, name: &str, query: Option<&str>) {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id = fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
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
    }

    /// A member that carries a platform presence.
    async fn attach_link(
        db: &crate::database::DbHandle,
        id: i32,
        name: &str,
        platform: &str,
        handle: &str,
    ) {
        let user_id = fixtures::user(db, "project").await;
        let capture_id = fixtures::capture(db, user_id).await;
        let illumination_id = fixtures::illumination(db, user_id, capture_id).await;
        let observation_id = fixtures::entity(db, user_id, capture_id, illumination_id, 0, name).await;
        observation::Entity::update_many()
            .col_expr(observation::Column::Platform, Expr::value(Some(platform.to_string())))
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
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Tsoding").await;
        attach_link(&db, id, "Tsoding", "x_twitter", "tsoding").await;

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.platform.as_deref(), Some("x_twitter"));
        assert_eq!(entity.platform_handle.as_deref(), Some("tsoding"));
        assert_eq!(entity.platform_display_name, None);
    }

    /// The hint is a property of the entity, projected like the description: one selected, the rest
    /// kept, in member order so the answer never depends on arrival order.
    #[tokio::test]
    async fn the_hint_is_projected_with_its_alternatives() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        attach(&db, id, "Train Dreams", Some("Train Dreams novella")).await;
        attach(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson")).await;

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.search_query.as_deref(), Some("Train Dreams novella"));
        assert_eq!(
            entity.alt_search_queries,
            vec!["Train Dreams Denis Johnson"],
            "the selected hint is never one of its own alternatives"
        );
    }

    /// A selected hint is not silently swapped for another member's while its own member still
    /// carries it — the same protection the description has.
    #[tokio::test]
    async fn a_selected_hint_survives_while_a_member_still_carries_it() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        attach(&db, id, "Train Dreams", Some("Train Dreams novella")).await;
        attach(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson")).await;

        entity::Entity::update(entity::ActiveModel {
            id: Set(id),
            search_query: Set(Some("Train Dreams Denis Johnson".to_string())),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap();

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(
            entity.search_query.as_deref(),
            Some("Train Dreams Denis Johnson"),
            "a hint a member still carries must not be replaced"
        );
    }

    #[tokio::test]
    async fn a_live_claim_makes_it_identified() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "claim", "wikidata", "Q104919").await;

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.status, "identified");
        assert!(entity.confidence.is_some(), "a claim carries a confidence");
    }

    #[tokio::test]
    async fn a_collision_makes_it_contested() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "collision", "wikidata", "Q104919").await;

        project(&db.conn, id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.status, "contested");
        assert_eq!(entity.confidence, None, "a collision is not an identity");
    }

    #[tokio::test]
    async fn a_retracted_claim_does_not_identify() {
        let Some(db) = test_db::test_db().await else { return };
        let db = db.handle();
        let id = seed(&db, "Train Dreams").await;
        claim(&db, id, "claim", "wikidata", "Q104919").await;
        evidence::Entity::update_many()
            .col_expr(evidence::Column::RetractedAt, Expr::value(chrono::Utc::now()))
            .filter(evidence::Column::KnowledgeEntityId.eq(id))
            .exec(&db.conn)
            .await
            .unwrap();

        project(&db.conn, id).await.unwrap();

        assert_eq!(load(&db, id).await.status, "provisional");
    }
}
