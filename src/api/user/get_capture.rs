use std::collections::HashMap;

use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

// These are user-specific, so they take a context and only return captures belonging to that user.
//
// Archiving hides a capture from feeds and timelines; it does not remove it. Bulk
// loads therefore skip archived captures, while a lookup by id includes them: the
// owner asked for that capture specifically, and the detail page needs to render
// it so it can be unarchived.

pub async fn get_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    load_captures(db, context, ids, false).await
}

/// Internal hydration path for callers that must see archived rows (the archived
/// listing). The feed-facing `get_captures` deliberately filters them out.
pub(super) async fn get_captures_including_archived(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    load_captures(db, context, ids, true).await
}

/// Fetch one of the user's own captures by id, archived included.
pub async fn get_capture(
    db: &DbHandle,
    context: &auth::Context,
    id: i32,
) -> Result<model::capture::ModelEx, ApiError> {
    load_captures(db, context, vec![id], true)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::not_found(anyhow!("Capture with id {id} not found")))
}

async fn load_captures(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
    include_archived: bool,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if ids.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "capture_ids must contain at least one capture ID"
        )));
    }

    let mut loader = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::Id.is_in(ids.clone()));

    if !include_archived {
        loader = loader.filter(model::capture::Column::ArchivedAt.is_null());
    }

    let captures = loader
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::annotation::Entity)
        .with((model::illumination::Entity, model::user_observed_entity::Entity))
        .all(&db.conn)
        .await?;

    Ok(order_captures(captures, ids))
}

/// Thumbnail/summary hydration: same ownership and archive rules as `get_captures`,
/// without annotations or knowledge entities.
pub(super) async fn get_captures_for_previews(
    db: &DbHandle,
    context: &auth::Context,
    ids: Vec<i32>,
) -> Result<Vec<model::capture::ModelEx>, ApiError> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::UserId.eq(context.user_id()))
        .filter(model::capture::Column::ArchivedAt.is_null())
        .filter(model::capture::Column::Id.is_in(ids.clone()))
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .all(&db.conn)
        .await?;

    Ok(order_captures(captures, ids))
}

fn order_captures(
    captures: Vec<model::capture::ModelEx>,
    ids: Vec<i32>,
) -> Vec<model::capture::ModelEx> {
    let mut captures_by_id: HashMap<i32, model::capture::ModelEx> = captures
        .into_iter()
        .map(|capture| (capture.id, capture))
        .collect();

    ids.into_iter()
        .filter_map(|capture_id| captures_by_id.remove(&capture_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::user::get_captures_archived;
    use crate::test_support::{db_or_skip, fixtures};

    #[tokio::test]
    async fn archived_captures_are_hidden_from_bulk_loads_but_loadable_by_id() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "archive_ui").await;
        let active = fixtures::capture(&db, user).await;
        let archived = fixtures::archived_capture(&db, user).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));

        let bulk = get_captures(&db, &context, vec![active, archived])
            .await
            .unwrap();
        assert_eq!(bulk.len(), 1);
        assert_eq!(bulk[0].id, active);

        let by_id = get_capture(&db, &context, archived).await.unwrap();
        assert_eq!(by_id.id, archived);

        let archived_list = get_captures_archived(&db, &context, 50).await.unwrap();
        assert_eq!(archived_list.len(), 1);
        assert_eq!(archived_list[0].id, archived);
    }

    /// Feed hydration answers in the order the caller asked for: the ids come from a ranked
    /// view, and a database-ordered result would scramble it.
    #[tokio::test]
    async fn bulk_load_preserves_the_requested_order() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "capture_order").await;
        let first = fixtures::capture(&db, user).await;
        let second = fixtures::capture(&db, user).await;
        let third = fixtures::capture(&db, user).await;

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let ordered = get_captures(&db, &context, vec![third, first, second])
            .await
            .unwrap();

        assert_eq!(
            ordered.iter().map(|capture| capture.id).collect::<Vec<_>>(),
            vec![third, first, second]
        );
    }

    /// Rows written before `suggested_searches` existed carry NULL; hydration must still serve them.
    #[tokio::test]
    async fn capture_loading_accepts_legacy_null_suggested_searches() {
        db_or_skip!(db);
        let user = fixtures::user(&db, "legacy_illum").await;
        let capture = fixtures::capture(&db, user).await;
        model::illumination::ActiveModel::builder()
            .set_user_id(user)
            .set_capture_id(capture)
            .set_task_run_id(uuid::Uuid::new_v4().to_string())
            .set_summary("legacy summary")
            .set_details("")
            .set_suggested_searches(None)
            .insert(&db.conn)
            .await
            .unwrap();

        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user));
        let captures = get_captures(&db, &context, vec![capture]).await.unwrap();
        assert_eq!(captures.len(), 1);
        let HasMany::Loaded(illuminations) = &captures[0].illuminations else {
            panic!("illuminations should be loaded");
        };
        assert_eq!(illuminations.len(), 1);
        assert_eq!(illuminations[0].suggested_searches, None);
    }
}
