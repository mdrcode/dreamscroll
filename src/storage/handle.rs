use uuid::Uuid;

use crate::{api, model};

#[derive(Debug, Clone)]
pub struct StorageHandle {
    pub provider: String,

    pub bucket: Option<String>,
    pub user_shard: String,
    pub uuid: Uuid,
    pub extension: Option<String>,
}

impl std::fmt::Display for StorageHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uuid {} (p:{} b:{})",
            self.uuid,
            self.provider,
            self.bucket.as_deref().unwrap_or("")
        )
    }
}

impl From<&model::media::ModelEx> for StorageHandle {
    fn from(media: &model::media::ModelEx) -> Self {
        StorageHandle {
            provider: media.storage_provider.clone(),
            bucket: media.storage_bucket.clone(),
            user_shard: media.storage_user_shard.clone(),
            uuid: media.storage_uuid.clone(),
            extension: media.storage_extension.clone(),
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
            extension: media.storage_extension.clone(),
        }
    }
}