use sea_orm::{EntityLoaderTrait, prelude::*};
use sea_orm::{EntityTrait, QuerySelect};

use crate::{api, auth, common::AppError, database::DbHandle, entity::*};

pub async fn search_by_illuminations(
    user_context: auth::Context,
    db: &DbHandle,
    query: &str,
) -> anyhow::Result<Vec<api::CaptureInfo>, AppError> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    // Start from captures filtered by user (indexed), then join to illuminations
    let capture_ids_with_match = capture::Entity::find()
        .filter(capture::Column::UserId.eq(user_context.user_id()))
        .inner_join(illumination::Entity)
        .filter(illumination::Column::Content.contains(query))
        .column(capture::Column::Id)
        .distinct()
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|c| c.id)
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
