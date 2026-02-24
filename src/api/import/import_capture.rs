use std::path::PathBuf;

use anyhow::anyhow;
use chrono::Utc;
use sea_orm::{TryIntoModel, prelude::*};

use crate::{api::*, auth, database::DbHandle, model, storage};

pub async fn import_capture(
    db: &DbHandle,
    storage: &Box<dyn storage::StorageProvider>,
    user_context: &auth::Context,
    created_at: chrono::DateTime<Utc>,
    path: &PathBuf,
) -> Result<model::capture::ModelEx, ApiError> {
    let bytes = tokio::fs::read(path).await?;

    if !infer::is_image(&bytes) {
        return Err(ApiError::bad_request(anyhow!(
            "Uploaded media is not an image."
        )));
    }

    // During the import flow, we prevent duplicate imports as a convenience.
    // This is *not* enforced during api::user::insert_capture.
    let hash_blake3 = blake3::hash(&bytes);
    if model::media::Entity::find()
        .filter(model::media::Column::HashBlake3.eq(hash_blake3.to_hex().to_string()))
        .one(&db.conn)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(anyhow!(
            "An image with the same content hash already exists in the database."
        )));
    }

    let media_type = infer::get(&bytes).ok_or_else(|| anyhow!("Could not infer media type."))?;
    let bytes_len = bytes.len();

    let handle = storage
        .store_bytes(
            bytes::Bytes::from(bytes),
            user_context.storage_shard(),
            Some(media_type.extension()),
        )
        .await?;

    let media_builder = model::media::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_bytes(bytes_len as i64)
        .set_mime_type(Some(media_type.mime_type().to_string()))
        .set_hash_blake3(Some(hash_blake3.to_hex().to_string()))
        .set_storage_provider(handle.provider)
        .set_storage_bucket(handle.bucket)
        .set_storage_user_shard(handle.user_shard)
        .set_storage_uuid(handle.uuid)
        .set_storage_extension(handle.extension);

    let capture = model::capture::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_created_at(created_at)
        .add_media(media_builder)
        .save(&db.conn)
        .await?;

    Ok(capture.try_into_model()?)
}
