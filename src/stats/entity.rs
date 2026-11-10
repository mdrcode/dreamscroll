//! One knowledge entity as evidence: its members and claims, for review and audit.
//!
//! `eval inspect` answers "why did the resolver decide this". This answers "what is this entity
//! made of" — the observations it absorbed and the identities it asserts, including retracted ones.

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::database;
use crate::model;

pub struct Detail {
    pub entity: model::knowledge_entity::Model,
    pub members: Vec<model::user_observed_entity::Model>,
    pub claims: Vec<ClaimRow>,
}

pub struct ClaimRow {
    pub id: i32,
    pub scheme: String,
    pub external_id: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub classes: Vec<String>,
    pub confidence: Option<f32>,
    pub method: Option<String>,
    pub asserted_by: String,
    pub basis: String,
    pub surface: Option<String>,
    pub retracted_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub user_observed_entity_id: Option<i32>,
    pub source_user_id: Option<i32>,
}

pub async fn detail(db: &database::DbHandle, id: i32) -> anyhow::Result<Option<Detail>> {
    let Some(entity) = model::knowledge_entity::Entity::find_by_id(id)
        .one(&db.conn)
        .await?
    else {
        return Ok(None);
    };

    let members = model::user_observed_entity::Entity::find()
        .filter(model::user_observed_entity::Column::KnowledgeEntityId.eq(id))
        .order_by_asc(model::user_observed_entity::Column::CreatedAt)
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?;

    let claims = model::knowledge_claims::Entity::find()
        .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
        .order_by_asc(model::knowledge_claims::Column::Id)
        .find_with_related(model::knowledge_identity::Entity)
        .all(&db.conn)
        .await?
        .into_iter()
        .filter_map(|(claim, identities)| {
            let identity = identities.into_iter().next()?;
            Some(ClaimRow {
                id: claim.id,
                scheme: identity.scheme,
                external_id: identity.external_id,
                label: identity.label,
                description: identity.description,
                classes: identity.classes,
                confidence: identity.confidence,
                method: identity.method,
                asserted_by: identity.asserted_by,
                basis: claim.basis.as_str().to_string(),
                surface: claim.surface,
                retracted_at: claim.retracted_at,
                observed_at: claim.observed_at,
                user_observed_entity_id: claim.user_observed_entity_id,
                source_user_id: claim.source_user_id,
            })
        })
        .collect();

    Ok(Some(Detail {
        entity,
        members,
        claims,
    }))
}

impl Detail {
    pub fn to_json(&self) -> serde_json::Value {
        let entity = &self.entity;
        let members: Vec<serde_json::Value> = self
            .members
            .iter()
            .map(|member| {
                serde_json::json!({
                    "id": member.id,
                    "capture_id": member.capture_id,
                    "illumination_id": member.illumination_id,
                    "position": member.position,
                    "created_at": member.created_at.to_rfc3339(),
                    "name": member.name,
                    "entity_type": member.entity_type,
                    "description": member.description,
                    "search_query": member.search_query,
                    "platform": member.platform,
                    "platform_handle": member.platform_handle,
                    "platform_display_name": member.platform_display_name,
                })
            })
            .collect();
        let claims: Vec<serde_json::Value> = self
            .claims
            .iter()
            .map(|claim| {
                serde_json::json!({
                    "id": claim.id,
                    "scheme": claim.scheme,
                    "external_id": claim.external_id,
                    "label": claim.label,
                    "description": claim.description,
                    "classes": claim.classes,
                    "confidence": claim.confidence,
                    "method": claim.method,
                    "asserted_by": claim.asserted_by,
                    "basis": claim.basis,
                    "surface": claim.surface,
                    "retracted_at": claim.retracted_at.map(|at| at.to_rfc3339()),
                    "observed_at": claim.observed_at.to_rfc3339(),
                    "user_observed_entity_id": claim.user_observed_entity_id,
                    "source_user_id": claim.source_user_id,
                })
            })
            .collect();

        serde_json::json!({
            "entity": {
                "id": entity.id,
                "name": entity.name,
                "alt_names": entity.alt_names,
                "entity_type": entity.entity_type,
                "alt_types": entity.alt_types,
                "description": entity.description,
                "alt_descriptions": entity.alt_descriptions,
                "search_query": entity.search_query,
                "alt_search_queries": entity.alt_search_queries,
                "platform": entity.platform,
                "platform_handle": entity.platform_handle,
                "platform_display_name": entity.platform_display_name,
                "status": entity.status.as_str(),
                "confidence": entity.confidence,
                "merged_into_id": entity.merged_into_id,
                "origin_observation_id": entity.origin_observation_id,
            },
            "members": members,
            "claims": claims,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn detail_lists_members_and_claims() {
        db_or_skip!(db);
        let id = fixtures::knowledge_entity_with_member(&db, "Entity Detail").await;
        fixtures::claim(
            &db,
            id,
            "wikidata",
            "Q1",
            "wikidata",
            Some("exact_label"),
            Some(0.9),
            model::knowledge_claims::ClaimBasis::Text,
        )
        .await;

        let detail = detail(&db, id).await.unwrap().unwrap();
        assert_eq!(detail.entity.id, id);
        assert_eq!(detail.members.len(), 1);
        assert_eq!(detail.claims.len(), 1);
        assert_eq!(detail.claims[0].scheme, "wikidata");
        assert_eq!(detail.claims[0].external_id, "Q1");
        assert_eq!(detail.claims[0].basis, "text");

        let json = detail.to_json();
        assert_eq!(
            json["entity"]["origin_observation_id"].as_i64(),
            detail.entity.origin_observation_id.map(i64::from)
        );
        assert_eq!(json["claims"][0]["external_id"], "Q1");
    }
}
