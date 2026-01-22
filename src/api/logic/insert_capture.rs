use chrono::Utc;
use sea_orm::TryIntoModel;

use crate::{api, auth, database::DbHandle, model};

pub async fn insert_capture(
    user_context: auth::Context,
    db: &DbHandle,
    storage_id: String,
) -> Result<api::CaptureInfo, api::ApiError> {
    let media = model::media::ActiveModel::builder().set_filename(storage_id.clone());

    let active_model = model::capture::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_created_at(Utc::now())
        .add_media(media)
        .save(&db.conn)
        .await?;

    let model = active_model.try_into_model()?;

    Ok(api::CaptureInfo::from(model))
}
