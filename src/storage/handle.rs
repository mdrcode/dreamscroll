use uuid::Uuid;

use crate::{api, model, storage};

#[derive(Debug, Clone)]
pub struct StorageHandle {
    pub provider: String,

    pub bucket: Option<String>,
    pub user_shard: Option<String>,
    pub uuid: Uuid,
}

impl std::fmt::Display for StorageHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "uuid {} ({})", self.uuid, self.provider,)
    }
}

impl From<storage::StorageHandle> for model::media::ActiveModelEx {
    fn from(storage_id: storage::StorageHandle) -> Self {
        model::media::ActiveModel::builder()
            .set_storage_provider(storage_id.provider)
            .set_storage_bucket(storage_id.bucket)
            .set_storage_user_shard(storage_id.user_shard)
            .set_storage_uuid(storage_id.uuid)
    }
}

impl From<&model::media::ModelEx> for StorageHandle {
    fn from(media: &model::media::ModelEx) -> Self {
        StorageHandle {
            provider: media.storage_provider.clone(),
            bucket: media.storage_bucket.clone(),
            user_shard: media.storage_user_shard.clone(),
            uuid: media.storage_uuid.clone(),
        }
    }
}

impl From<&api::MediaInfo> for StorageHandle {
    fn from(media: &api::MediaInfo) -> Self {
        StorageHandle {
            provider: media.storage_provider.clone(),
            bucket: media.storage_bucket.clone(),
            user_shard: media.storage_shard.clone(),
            uuid: media.storage_uuid,
        }
    }
}
