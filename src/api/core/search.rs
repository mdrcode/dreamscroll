use sea_orm::{EntityLoaderTrait, EntityTrait, QuerySelect, prelude::*};

use crate::{api, auth, database::DbHandle, model};

#[tracing::instrument(skip(db, user_context))]
pub async fn search_by_illuminations(
    db: &DbHandle,
    user_context: &auth::Context,
    query: &str,
) -> anyhow::Result<Vec<model::capture::ModelEx>, api::ApiError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    let capture_ids_with_match = model::search_index::Entity::find()
        .filter(model::search_index::Column::UserId.eq(user_context.user_id()))
        .filter(model::search_index::Column::Content.contains(query))
        .column(model::search_index::Column::CaptureId)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|si| si.capture_id)
        .collect::<Vec<i32>>();

    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::Id.is_in(capture_ids_with_match))
        .order_by_id_desc()
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .with(model::xquery::Entity)
        .with(model::knode::Entity)
        .with(model::social_media::Entity)
        .all(&db.conn)
        .await?;

    Ok(captures)
}

pub async fn get_capture_ids_missing_search(
    db: &DbHandle,
    user_context: &auth::Context,
) -> anyhow::Result<Vec<i32>, api::ApiError> {
    let captures_without_index = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .left_join(model::search_index::Entity)
        .filter(model::search_index::Column::Id.is_null())
        .column(model::capture::Column::Id)
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect::<Vec<i32>>();

    Ok(captures_without_index)
}
