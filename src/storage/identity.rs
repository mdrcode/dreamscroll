use crate::{api, model, storage};

#[derive(Debug, Clone)]
pub struct StorageIdentity {
    pub storage_provider: String,

    pub provider_bucket: Option<String>,
    pub provider_shard: Option<String>,
    pub provider_id: String,
}

impl std::fmt::Display for StorageIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "provider_id {} ({})",
            self.provider_id, self.storage_provider,
        )
    }
}

impl From<storage::StorageIdentity> for model::media::ActiveModelEx {
    fn from(storage_id: storage::StorageIdentity) -> Self {
        model::media::ActiveModel::builder()
            .set_storage_provider(storage_id.storage_provider)
            .set_storage_bucket(storage_id.provider_bucket)
            .set_storage_shard(storage_id.provider_shard)
            .set_storage_id(storage_id.provider_id)
    }
}

impl From<&model::media::ModelEx> for StorageIdentity {
    fn from(media: &model::media::ModelEx) -> Self {
        StorageIdentity {
            storage_provider: media.storage_provider.clone(),
            provider_bucket: media.storage_bucket.clone(),
            provider_shard: media.storage_shard.clone(),
            provider_id: media.storage_id.clone(),
        }
    }
}

impl From<&api::MediaInfo> for StorageIdentity {
    fn from(media: &api::MediaInfo) -> Self {
        StorageIdentity {
            storage_provider: media.storage_provider.clone(),
            provider_bucket: media.storage_bucket.clone(),
            provider_shard: media.storage_shard.clone(),
            provider_id: media.storage_id.clone(),
        }
    }
}
