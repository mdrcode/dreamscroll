use anyhow::anyhow;
use sea_orm::prelude::*;

use crate::{api, database, model, storage};

pub async fn get_media_for_capture(
    db: &database::DbHandle,
    info_maker: &api::InfoMaker,
    capture_id: i32,
) -> Result<Vec<(api::MediaInfo, storage::StorageIdentity)>, api::ApiError> {
    let media_models = model::media::Entity::find()
        .filter(model::media::Column::CaptureId.eq(capture_id))
        .all(&db.conn)
        .await?;

    if media_models.is_empty() {
        return Err(api::ApiError::not_found(anyhow!(
            "No media found for capture ID {}",
            capture_id
        )));
    }

    let media_infos: Vec<(api::MediaInfo, storage::StorageIdentity)> = media_models
        .into_iter()
        .map(|m| {
            let model_ex: model::media::ModelEx = m.into();
            (
                info_maker.make_media_info(&model_ex),
                storage::StorageIdentity::from(&model_ex),
            )
        })
        .collect();

    Ok(media_infos)
}
