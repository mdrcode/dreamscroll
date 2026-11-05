//! Projection: the derived columns of a knowledge entity.
//!
//! One function writes `name`, `alt_names`, `description`, `alt_descriptions`, `search_query`,
//! `alt_search_queries`, `platform`, `platform_handle`, `platform_display_name`, `status` and
//! `confidence`, and nothing else does. The confidence comes from the store's [`Worth`]: how good a
//! claim is, and how much an extra independent input adds, is policy, not projection. It reads the entity's members and its evidence and derives
//! every column from them, so it is safe to run at any time and twice in a row.
//!
//! The entity **wears the values of its origin** — the member it was created from — and every other
//! member contributes alternatives. That is what makes a merge safe: the winner keeps its own surface
//! and text while the loser's arrive as `alt_*`, without anyone having to guess which of them the
//! current columns came from.
//!
//! Callers reach this through [`Knowledge::update_projection`]. The functions below are `pub(super)`
//! for one reason: `merge` projects inside its transaction and so needs the connection-agnostic form.

use std::collections::HashSet;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder, Set,
    Statement,
};

use crate::model;

use super::evidence::ClaimBasis;
use super::{STATUS_IDENTIFIED, STATUS_PROVISIONAL};

/// Recompute the columns that are a function of the members and of the evidence — the only writer of
/// them after `create`.
///
/// The entity **wears the values of its origin** and every other member contributes alternatives,
/// which is what makes a merge safe: the winner keeps its own surface and text.
pub(super) async fn project<C: ConnectionTrait>(
    conn: &C,
    knowledge_entity_id: i32,
) -> anyhow::Result<model::knowledge_entity::Model> {
    let row = super::graph::get(conn, knowledge_entity_id).await?;

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
        |member| surfaces(member).collect(),
        super::normalize::comparable,
    );
    let types = gather(
        &members,
        |member| vec![member.entity_type.as_str()],
        str::to_string,
    );
    let descriptions = gather(
        &members,
        |member| member.description.as_deref().into_iter().collect(),
        str::to_string,
    );
    let queries = gather(
        &members,
        |member| member.search_query.as_deref().into_iter().collect(),
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

    // A claim is the edge; who answered for the identity, and how surely, is on the identity.
    let claim_rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT c.basis, i.confidence \
               FROM knowledge_claims c \
               JOIN knowledge_identities i ON i.id = c.knowledge_identity_id \
              WHERE c.knowledge_entity_id = $1 AND c.retracted_at IS NULL",
            [knowledge_entity_id.into()],
        ))
        .await?;
    let claims: Vec<(ClaimBasis, Option<f32>)> = claim_rows
        .iter()
        .map(|row| {
            Ok((
                ClaimBasis::from_db(&row.try_get_by_index::<String>(0)?),
                row.try_get_by_index::<Option<f32>>(1)?,
            ))
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()?;
    // Status is one axis: how well substantiated the entity is. Whether it shares an identity with
    // another entity is a different question, answered by `knowledge_proximities` at pair granularity —
    // a third status value conflated the two and had to be re-projected on every scan.
    let status = if claims.is_empty() {
        STATUS_PROVISIONAL
    } else {
        STATUS_IDENTIFIED
    };

    let confidence = super::heuristic::scoring::combine(&claims);

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

fn surfaces(observation: &model::user_observed_entity::Model) -> impl Iterator<Item = &str> {
    [
        Some(observation.name.as_str()),
        observation.platform_handle.as_deref(),
        observation.platform_display_name.as_deref(),
    ]
    .into_iter()
    .flatten()
}

/// Every distinct, non-empty value the members offer, in member order.
fn gather<'a>(
    members: &'a [model::user_observed_entity::Model],
    values: impl Fn(&'a model::user_observed_entity::Model) -> Vec<&'a str>,
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

#[cfg(test)]
mod tests {
    use super::*;

    use sea_orm::sea_query::Expr;

    use crate::database::DbHandle;
    use crate::model::{knowledge_entity as entity, user_observed_entity as observation};
    use crate::test_support::{fixtures, test_db};

    fn store(db: &DbHandle) -> crate::knowledge::Knowledge {
        crate::knowledge::Knowledge::new(db)
    }

    async fn claim(db: &DbHandle, id: i32, scheme: &str, value: &str) {
        fixtures::claim(db, id, scheme, value, "test", None, None, "text").await;
    }

    async fn proximity(db: &DbHandle, holder: i32, scheme: &str, value: &str) {
        let claimant = fixtures::knowledge_entity_with_member(db, "a claimant").await;
        let identity = crate::knowledge::evidence::Evidence::from_handle(db)
            .upsert_identity(&crate::knowledge::evidence::DraftIdentity {
                scheme: scheme.to_string(),
                external_id: value.to_string(),
                source: "test".to_string(),
                method: None,
                confidence: None,
                label: None,
                description: None,
                classes: Vec::new(),
            })
            .await
            .unwrap();
        model::knowledge_proximities::Entity::insert(model::knowledge_proximities::ActiveModel {
            holder_id: Set(holder),
            counterpart_id: Set(claimant),
            knowledge_identity_id: Set(Some(identity)),
            basis: Set(crate::knowledge::evidence::Basis::SharedId
                .as_str()
                .to_string()),
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
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;

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

    /// Another member of an existing entity: a capture whose row carries `hint` and/or a presence.
    async fn attach(
        db: &DbHandle,
        id: i32,
        name: &str,
        hint: Option<&str>,
        platform: Option<(&str, &str)>,
    ) -> i32 {
        let capture = fixtures::Capture::new(db, "project_member").await;
        let observation_id = fixtures::observe(
            db,
            &capture,
            0,
            name,
            None,
            "book",
            platform.map(|(platform, _)| platform),
            platform.map(|(_, handle)| handle),
        )
        .await;
        observation::Entity::update_many()
            .col_expr(
                observation::Column::SearchQuery,
                Expr::value(hint.map(str::to_string)),
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

    /// The platform presence, projected from whichever member carried one.
    #[tokio::test]
    async fn the_platform_presence_is_projected_from_a_member() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = fixtures::knowledge_entity_with_member(&db, "Tsoding").await;
        attach(&db, id, "Tsoding", None, Some(("x_twitter", "tsoding"))).await;

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
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attach(&db, id, "Train Dreams", Some("Train Dreams novella"), None).await;
        attach(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson"), None).await;

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
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        attach(&db, id, "Train Dreams", Some("Train Dreams novella"), None).await;
        let second = attach(&db, id, "Train Dreams", Some("Train Dreams Denis Johnson"), None).await;

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
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        claim(&db, id, "wikidata", "Q104919").await;

        store(&db).update_projection(id).await.unwrap();

        let entity = load(&db, id).await;
        assert_eq!(entity.status, STATUS_IDENTIFIED);
        assert!(entity.confidence.is_some(), "a claim carries a confidence");
    }

    /// Sharing an identity with another entity is a separate question from how well substantiated
    /// this one is, so it must not leak into the status.
    #[tokio::test]
    async fn a_proximity_does_not_change_the_status() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        proximity(&db, id, "wikidata", "Q104919").await;

        store(&db).update_projection(id).await.unwrap();

        assert_eq!(load(&db, id).await.status, STATUS_PROVISIONAL);
    }

    #[tokio::test]
    async fn a_retracted_claim_does_not_identify() {
        let Some(db) = test_db::test_db().await else {
            return;
        };
        let db = db.handle();
        let id = fixtures::knowledge_entity_with_member(&db, "Train Dreams").await;
        claim(&db, id, "wikidata", "Q104919").await;
        model::knowledge_claims::Entity::update_many()
            .col_expr(
                model::knowledge_claims::Column::RetractedAt,
                Expr::value(chrono::Utc::now()),
            )
            .filter(model::knowledge_claims::Column::KnowledgeEntityId.eq(id))
            .exec(&db.conn)
            .await
            .unwrap();

        store(&db).update_projection(id).await.unwrap();

        assert_eq!(load(&db, id).await.status, STATUS_PROVISIONAL);
    }
}
