use chrono::Utc;
use sea_orm::TryIntoModel;

use crate::{api, database::DbHandle, model, storage};

pub async fn import_capture(
    db: &DbHandle,
    user_id: i32,
    media1: storage::StorageIdentity,
    created_at: chrono::DateTime<Utc>,
) -> Result<model::capture::ModelEx, api::ApiError> {
    let media = model::media::ActiveModelEx::from(media1);

    let active_model = model::capture::ActiveModel::builder()
        .set_user_id(user_id)
        .set_created_at(created_at)
        .add_media(media)
        .save(&db.conn)
        .await?;

    Ok(active_model.try_into_model()?)
}
