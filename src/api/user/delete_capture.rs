use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api::*, auth, database::DbHandle, model};

pub async fn delete_capture(
    db: &DbHandle,
    context: &auth::Context,
    capture_id: i32,
) -> Result<(), ApiError> {
    let capture = model::capture::Entity::find_by_id(capture_id)
        .one(&db.conn)
        .await?;

    let Some(capture) = capture else {
        return Err(ApiError::not_found(anyhow!(
            "Capture with id {} not found",
            capture_id
        )));
    };

    if capture.user_id != context.user_id() {
        return Err(ApiError::not_found(anyhow!(
            "Capture with id {} not found or access denied",
            capture_id
        )));
    }

    let illumination_ids = model::illumination::Entity::find()
        .filter(model::illumination::Column::UserId.eq(context.user_id()))
        .filter(model::illumination::Column::CaptureId.eq(capture_id))
        .all(&db.conn)
        .await?
        .into_iter()
        .map(|illumination| illumination.id)
        .collect::<Vec<i32>>();

    if !illumination_ids.is_empty() {
        model::illumination_meta::Entity::delete_many()
            .filter(model::illumination_meta::Column::UserId.eq(context.user_id()))
            .filter(model::illumination_meta::Column::IlluminationId.is_in(illumination_ids))
            .exec(&db.conn)
            .await?;
    }

    model::search_index::Entity::delete_many()
        .filter(model::search_index::Column::UserId.eq(context.user_id()))
        .filter(model::search_index::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::xquery::Entity::delete_many()
        .filter(model::xquery::Column::UserId.eq(context.user_id()))
        .filter(model::xquery::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::knode::Entity::delete_many()
        .filter(model::knode::Column::UserId.eq(context.user_id()))
        .filter(model::knode::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::social_media::Entity::delete_many()
        .filter(model::social_media::Column::UserId.eq(context.user_id()))
        .filter(model::social_media::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::illumination::Entity::delete_many()
        .filter(model::illumination::Column::UserId.eq(context.user_id()))
        .filter(model::illumination::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::media::Entity::delete_many()
        .filter(model::media::Column::UserId.eq(context.user_id()))
        .filter(model::media::Column::CaptureId.eq(capture_id))
        .exec(&db.conn)
        .await?;

    model::capture::Entity::delete_by_id(capture_id)
        .exec(&db.conn)
        .await?;

    Ok(())
}
