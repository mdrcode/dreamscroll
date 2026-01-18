use chrono::Utc;
use sea_orm::TryIntoModel;

use crate::{api, common::*, database::DbHandle, entity};

pub async fn insert_capture(
    db: &DbHandle,
    storage_id: String,
) -> anyhow::Result<api::CaptureInfo, AppError> {
    let media = entity::media::ActiveModel::builder().set_filename(storage_id.clone());

    let active_model = entity::capture::ActiveModel::builder()
        .set_created_at(Utc::now())
        .add_media(media)
        .save(&db.conn)
        .await?;

    let model = active_model.try_into_model()?;

    Ok(api::CaptureInfo::from(model))
}
