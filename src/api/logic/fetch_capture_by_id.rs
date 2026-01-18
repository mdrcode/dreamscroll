use anyhow::anyhow;
use sea_orm::prelude::*;
use sea_orm::{EntityLoaderTrait, EntityTrait, QueryOrder, QuerySelect};

use crate::{api, common::AppError, database::DbHandle, entity::*};

pub async fn fetch_capture_by_id(
    db: &DbHandle,
    id: i32,
) -> anyhow::Result<api::CaptureInfo, AppError> {
    let capture = capture::Entity::load()
        .filter_by_id(id)
        .with(media::Entity)
        .with(illumination::Entity)
        .one(&db.conn)
        .await
        .map_err(|e| AppError::internal(anyhow!("DB error fetching capture id {}: {}", id, e)))?;

    match capture {
        Some(capture) => Ok(api::CaptureInfo::new(capture)),
        None => Err(AppError::not_found(anyhow!("Capture id {} not found", id))),
    }
}
