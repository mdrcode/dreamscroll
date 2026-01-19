use sea_orm::{EntityLoaderTrait, prelude::*};
use sea_orm::{EntityTrait, QuerySelect};

use crate::{api, common::AppError, database::DbHandle, entity::*};

pub async fn search_by_illuminations(
    db: &DbHandle,
    query: &str,
) -> anyhow::Result<Vec<api::CaptureInfo>, AppError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    // Find illuminations that contain the search query
    let capture_ids_with_match = illumination::Entity::find()
        .filter(illumination::Column::Content.contains(query))
        .column(illumination::Column::CaptureId)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|i| i.capture_id)
        .collect::<Vec<i32>>();

    // Get unique capture IDs
    let captures = capture::Entity::load()
        .filter(capture::Column::Id.is_in(capture_ids_with_match))
        .order_by_id_desc()
        .with(media::Entity)
        .with(illumination::Entity)
        .all(&db.conn)
        .await?;

    let capture_info = captures
        .into_iter()
        .map(|model| api::CaptureInfo::from(model))
        .collect();

    Ok(capture_info)
}
