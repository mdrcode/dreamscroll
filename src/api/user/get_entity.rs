use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::QueryOrder;

use crate::{api::*, auth, database::DbHandle, model};

/// Fetch one of the user's unarchived knowledge entities by occurrence ID.
///
/// Ids are only ever handed out from current views, but a stale id can still be presented — so the
/// occurrence is checked against the capture's current illumination before it is returned. The user
/// never sees an illumination their capture no longer stands behind.
pub async fn get_knowledge_entity(
    db: &DbHandle,
    context: &auth::Context,
    id: i32,
) -> Result<model::user_observed_entity::ModelEx, ApiError> {
    let entity = model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::user_observed_entity::Column::Id.eq(id))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .one(&db.conn)
        .await?
        .ok_or_else(|| ApiError::not_found(anyhow!("Knowledge entity with id {id} not found")))?;

    Ok(entity.into())
}

/// The newest illumination of every capture, as a set of ids.
///
/// The graph keeps **every** illumination of a capture — an illumination can be wrong, and the
/// correct one is often the older reading — while the view shows exactly one. Illuminations are only
/// ever appended, so the highest id is the latest, and deriving that keeps a stored flag from
/// drifting.
///
/// Both reads below filter on this, so a capture that has been re-read surfaces only its latest
/// reading, whether through the list or through a stale id.
fn current_illumination() -> sea_orm::sea_query::SelectStatement {
    use model::illumination::Column;
    use sea_orm::sea_query::{Alias, Query};
    Query::select()
        .expr_as(Expr::col(Column::Id).max(), Alias::new("id"))
        .from(model::illumination::Entity)
        .group_by_col(Column::CaptureId)
        .to_owned()
}

/// The knowledge entity an observation was linked to, if it has been.
///
/// Read through the observation rather than by id: entities are shared and have no owner, so a
/// lookup by id alone would serve any user any entity. Walking in from a capture the caller owns is
/// the whole access rule.
pub async fn linked_knowledge_entity(
    db: &DbHandle,
    observation: &model::user_observed_entity::ModelEx,
) -> Result<Option<model::knowledge_entity::Model>, ApiError> {
    let Some(knowledge_entity_id) = observation.knowledge_entity_id else {
        return Ok(None);
    };
    Ok(
        model::knowledge_entity::Entity::find_by_id(knowledge_entity_id)
            .filter(model::knowledge_entity::Column::MergedIntoId.is_null())
            .one(&db.conn)
            .await?,
    )
}

/// Fetch the user's unarchived knowledge entities of the requested type.
///
/// Scoped to each capture's current illumination: the graph keeps every reading, the view keeps one.
pub async fn get_knowledge_entities_by_type(
    db: &DbHandle,
    context: &auth::Context,
    entity_type: &str,
) -> Result<Vec<model::user_observed_entity::Model>, ApiError> {
    Ok(model::user_observed_entity::Entity::find()
        .inner_join(model::capture::Entity)
        .filter(model::user_observed_entity::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(model::user_observed_entity::Column::EntityType.eq(entity_type))
        .filter(
            Expr::col(model::user_observed_entity::Column::IlluminationId)
                .in_subquery(current_illumination()),
        )
        .order_by_asc(model::user_observed_entity::Column::Id)
        .all(&db.conn)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{db_or_skip, fixtures};

    /// One active and one archived capture, each with one entity named after
    /// its capture's state.
    async fn seed(db: &crate::database::DbHandle) -> (i32, Vec<i32>) {
        let user = fixtures::user(db, "entity_get").await;
        let active = fixtures::capture(db, user).await;
        let archived = fixtures::archived_capture(db, user).await;

        let mut ids = Vec::new();
        for (capture_id, name) in [(active, "active entity"), (archived, "archived entity")] {
            let illumination = fixtures::illumination(db, user, capture_id).await;
            ids.push(fixtures::entity(db, user, capture_id, illumination, 0, name).await);
        }

        (user, ids)
    }

    /// A knowledge entity inserted directly, with the optional fields these tests vary.
    async fn knowledge_entity(
        db: &crate::database::DbHandle,
        name: &str,
        platform: Option<(&str, &str)>,
        merged_into: Option<i32>,
    ) -> i32 {
        let (platform, handle) = match platform {
            Some((platform, handle)) => (Some(platform.to_string()), Some(handle.to_string())),
            None => (None, None),
        };
        model::knowledge_entity::Entity::insert(model::knowledge_entity::ActiveModel {
            name: sea_orm::Set(name.to_string()),
            entity_type: sea_orm::Set("unknown".to_string()),
            status: sea_orm::Set(model::knowledge_entity::Status::Provisional),
            platform: sea_orm::Set(platform),
            platform_handle: sea_orm::Set(handle),
            merged_into_id: sea_orm::Set(merged_into),
            ..Default::default()
        })
        .exec(&db.conn)
        .await
        .unwrap()
        .last_insert_id as i32
    }

    #[tokio::test]
    async fn entity_type_listing_excludes_archived_captures() {
        db_or_skip!(db);
        let (user, _) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let entities = get_knowledge_entities_by_type(&db, &context, "unknown")
            .await
            .unwrap();
        assert_eq!(
            entities
                .into_iter()
                .map(|entity| entity.name)
                .collect::<Vec<_>>(),
            vec!["active entity"]
        );
    }

    /// The referent is served through the observation the caller owns, and carries the projected
    /// platform presence — the column the resolver now reads.
    #[tokio::test]
    async fn the_linked_entity_is_served_with_its_platform_presence() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "entity_presence").await;

        let knowledge_entity_id = knowledge_entity(
            &db,
            "active entity",
            Some(("x_twitter", "tsoding")),
            None,
        )
        .await;
        let observation_id = fixtures::attached(
            &db,
            &capture,
            0,
            "active entity",
            None,
            "unknown",
            knowledge_entity_id,
        )
        .await;

        let context =
            auth::Context::from(auth::DreamscrollAuthUser::new_test_session(capture.user_id));
        let observation = get_knowledge_entity(&db, &context, observation_id)
            .await
            .unwrap();
        let linked = linked_knowledge_entity(&db, &observation)
            .await
            .unwrap()
            .expect("the observation is linked");

        let info = KnowledgeEntityInfo::new(linked);
        assert_eq!(info.platform.as_deref(), Some("x_twitter"));
        assert_eq!(info.platform_handle.as_deref(), Some("tsoding"));
        assert_eq!(info.name, "active entity");
    }

    #[tokio::test]
    async fn entity_lookup_excludes_archived_captures() {
        db_or_skip!(db);
        let (user, entity_ids) = seed(&db).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let active = get_knowledge_entity(&db, &context, entity_ids[0])
            .await
            .unwrap();
        assert_eq!(active.name, "active entity");

        let archived = get_knowledge_entity(&db, &context, entity_ids[1])
            .await
            .unwrap_err();
        assert_eq!(archived.http_status_code, axum::http::StatusCode::NOT_FOUND);
    }

    /// The capture is re-read. Both readings are in the graph — `link` keeps history — but the user
    /// only ever sees the current one, whether they list by type or hold a stale id.
    #[tokio::test]
    async fn only_the_current_reading_is_visible() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "entity_current").await;
        let capture = fixtures::capture(&db, user).await;

        let stale_illumination = fixtures::illumination(&db, user, capture).await;
        let stale = fixtures::entity(
            &db,
            user,
            capture,
            stale_illumination,
            0,
            "superseded reading",
        )
        .await;
        let current_illumination = fixtures::illumination(&db, user, capture).await;
        let current = fixtures::entity(
            &db,
            user,
            capture,
            current_illumination,
            0,
            "current reading",
        )
        .await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        let listed = get_knowledge_entities_by_type(&db, &context, "unknown")
            .await
            .unwrap();
        assert_eq!(
            listed
                .into_iter()
                .map(|entity| entity.name)
                .collect::<Vec<_>>(),
            vec!["current reading"],
            "a superseded reading must never reach the listing"
        );

        let superseded = get_knowledge_entity(&db, &context, stale)
            .await
            .unwrap_err();
        assert_eq!(
            superseded.http_status_code,
            axum::http::StatusCode::NOT_FOUND
        );

        let live = get_knowledge_entity(&db, &context, current).await.unwrap();
        assert_eq!(live.name, "current reading");
    }

    /// A merged entity is a tombstone kept for history; the observation's consumer must get no
    /// referent rather than the absorbed entity's stale surface.
    #[tokio::test]
    async fn a_merged_entity_is_not_served() {
        db_or_skip!(db);
        let capture = fixtures::Capture::new(&db, "entity_merged").await;
        let survivor = knowledge_entity(&db, "survivor", None, None).await;
        let absorbed = knowledge_entity(&db, "absorbed", None, Some(survivor)).await;
        let observation_id = fixtures::attached(
            &db,
            &capture,
            0,
            "absorbed",
            None,
            "unknown",
            absorbed,
        )
        .await;

        let context =
            auth::Context::from(auth::DreamscrollAuthUser::new_test_session(capture.user_id));
        let observation = get_knowledge_entity(&db, &context, observation_id)
            .await
            .unwrap();

        assert_eq!(
            linked_knowledge_entity(&db, &observation).await.unwrap(),
            None,
            "a merged entity must not be served as the observation's referent"
        );
    }
}
