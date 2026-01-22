use anyhow::anyhow;
use sea_orm::EntityLoaderTrait;

use crate::{api, database::DbHandle, model};

pub async fn fetch_capture_by_id(
    db: &DbHandle,
    capture_id: i32,
) -> Result<api::CaptureInfo, api::ApiError> {
    let capture_model = model::capture::Entity::load()
        .filter_by_id(capture_id)
        .with(model::media::Entity)
        .with(model::illumination::Entity)
        .one(&db.conn)
        .await?;

    match capture_model {
        Some(model) => Ok(api::CaptureInfo::from(model)),
        None => Err(api::ApiError::not_found(anyhow!(
            "Capture id {} not found",
            capture_id
        ))),
    }
}
