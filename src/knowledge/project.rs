//! Projection: the entity columns that are a function of its members and its evidence.
//!
//! Nothing else writes `name`, `alt_names`, `entity_type`, `alt_types`, `description`,
//! `alt_descriptions`, `status` or `confidence`. Deriving them here rather than at write time is what
//! makes a merge safe: a merge re-points observations and projects the winner, so the loser's names
//! and descriptions are picked up from the members that moved rather than being lost with the row.
//!
//! No network, no policy — this reads rows and writes columns.

use std::collections::HashSet;

use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set};

use crate::model;

/// Recompute the columns that are a function of the members and of the evidence. Nothing else writes
/// `name`, `alt_names`, `description`, `alt_descriptions`, `status` or `confidence`.
///
/// Deriving the surface and text here rather than at write time is what makes a merge safe: a merge
/// re-points observations and projects the winner, so the loser's names and descriptions are picked
/// up from the members that moved rather than being lost with the deleted row.
pub async fn project(conn: &sea_orm::DatabaseConnection, knowledge_entity_id: i32) -> anyhow::Result<()> {
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
    let mut types: Vec<String> = Vec::new();
    for member in &members {
        if !types.contains(&member.entity_type) {
            types.push(member.entity_type.clone());
        }
        for surface in crate::knowledge::link::surfaces(member) {
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
    }

    let current = model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
        .one(conn)
        .await?;
    let (existing_name, existing_type, existing_description, existing_alt_descriptions) =
        match &current {
            Some(row) => (
                row.name.clone(),
                row.entity_type.clone(),
                row.description.clone(),
                row.alt_descriptions.clone(),
            ),
            None => (String::new(), String::new(), None, Vec::new()),
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
