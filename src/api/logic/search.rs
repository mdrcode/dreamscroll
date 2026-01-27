use anyhow::anyhow;
use sea_orm::{EntityLoaderTrait, EntityTrait, QuerySelect, prelude::*};

use crate::{api, auth, database::DbHandle, model};

pub async fn search_by_illuminations(
    user_context: auth::Context,
    db: &DbHandle,
    query: &str,
) -> anyhow::Result<Vec<api::CaptureInfo>, api::ApiError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    /* TODO
    // Start from captures filtered by user (indexed), then join to illuminations
    let capture_ids_with_match = model::capture::Entity::find()
        .filter(model::capture::Column::UserId.eq(user_context.user_id()))
        .inner_join(model::illumination::Entity)
        .filter(model::illumination::Column::RawContent.contains(query))
        .column(model::capture::Column::Id)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<i32>>();

    // Get unique capture IDs
    let captures = model::capture::Entity::load()
        .filter(model::capture::Column::Id.is_in(capture_ids_with_match))
        .order_by_id_desc()
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .all(&db.conn)
        .await?;

    let capture_info = captures
        .into_iter()
        .map(|model| api::CaptureInfo::from(model))
        .collect();

    Ok(capture_info)
    */
    Err(api::ApiError::internal(anyhow!(
        "Illumination search not yet implemented"
    )))
}
