use crate::{api, storage};

pub async fn get_media_bytes(
    stg: &Box<dyn storage::StorageProvider>,
    media: api::MediaInfo,
) -> Result<Vec<u8>, api::ApiError> {
    // TODO should this be a From impl?
    let stg_id = storage::StorageIdentity {
        storage_provider: media.storage_provider,
        provider_id: media.storage_id,
        provider_shard: media.storage_shard,
        provider_bucket: media.storage_bucket,
    };

    Ok(stg.retrieve_bytes(&stg_id).await?)
}
