use anyhow::anyhow;
use chrono::Utc;
use sea_orm::TryIntoModel;

use crate::{api::*, auth, database::DbHandle, model, storage};

pub async fn insert_capture(
    db: &DbHandle,
    storage: &Box<dyn storage::StorageProvider>,
    user_context: &auth::Context,
    media_bytes: &[u8],
) -> Result<model::capture::ModelEx, ApiError> {
    let user_id = user_context.user_id();
    let shard = user_context.storage_shard();

    if !infer::is_image(&media_bytes) {
        return Err(ApiError::bad_request(anyhow!(
            "Uploaded media is not an image."
        )));
    }

    let media_type =
        infer::get(&media_bytes).ok_or_else(|| anyhow!("Could not infer media type."))?;
    tracing::info!("Media type inferred as {}", media_type.mime_type());

    // Currently we compute a hash when storing media as a convenience to avoid re-importing
    // duplicates during development. We might remove this in the future or move hash
    // computation to an async background job if it becomes a bottleneck.
    let hash_blake3 = blake3::hash(&media_bytes);

    let handle = storage
        .store_bytes(&media_bytes, shard, Some(media_type.extension()))
        .await?;
    tracing::info!("Stored media user:{} handle: {:?}", user_id, &handle);

    let media_builder = model::media::ActiveModel::builder()
        .set_user_id(user_id)
        .set_bytes(media_bytes.len() as i64)
        .set_mime_type(Some(media_type.mime_type().to_string()))
        .set_hash_blake3(Some(hash_blake3.to_hex().to_string()))
        .set_storage_provider(handle.provider)
        .set_storage_bucket(handle.bucket)
        .set_storage_user_shard(handle.user_shard)
        .set_storage_uuid(handle.uuid)
        .set_storage_extension(handle.extension);

    let capture = model::capture::ActiveModel::builder()
        .set_user_id(user_context.user_id())
        .set_created_at(Utc::now())
        .add_media(media_builder)
        .save(&db.conn)
        .await?;

    let model = capture.try_into_model()?;

    Ok(model)
}
