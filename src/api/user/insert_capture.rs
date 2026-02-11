use chrono::Utc;
use sea_orm::TryIntoModel;

use crate::{api::*, auth, database::DbHandle, model, storage};

pub async fn insert_capture(
    db: &DbHandle,
    user_context: &auth::Context,
    media1: storage::StorageIdentity,
) -> Result<model::capture::ModelEx, ApiError> {
    let media = model::media::ActiveModelEx::from(media1);

    let active_model = model::capture::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_created_at(Utc::now())
        .add_media(media)
        .save(&db.conn)
        .await?;

    let model = active_model.try_into_model()?;

    Ok(model)
}
