use crate::{api, storage};

pub async fn get_storage_bytes(
    stg: &Box<dyn storage::StorageProvider>,
    stg_id: storage::StorageIdentity,
) -> Result<Vec<u8>, api::ApiError> {
    Ok(stg.retrieve_bytes(&stg_id).await?)
}
