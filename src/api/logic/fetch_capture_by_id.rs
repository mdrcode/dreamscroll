use anyhow::anyhow;
use sea_orm::EntityLoaderTrait;

use crate::{api, auth, common::AppError, database::DbHandle, entity::*};

pub async fn fetch_capture_by_id(
    db: &DbHandle,
    capture_id: i32,
) -> anyhow::Result<api::CaptureInfo, AppError> {
    let capture_model = capture::Entity::load()
        .filter_by_id(capture_id)
        .with(media::Entity)
        .with(illumination::Entity)
        .one(&db.conn)
        .await
        .map_err(|e| {
            AppError::internal(anyhow!(
                "DB error fetching capture id {}: {}",
                capture_id,
                e
            ))
        })?;

    match capture_model {
        Some(model) => Ok(api::CaptureInfo::from(model)),
        None => Err(AppError::not_found(anyhow!(
            "Capture id {} not found",
            capture_id
        ))),
    }
}
