use std::path::PathBuf;

use async_trait::async_trait;

use super::*;

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

#[derive(Debug, Clone)]
pub enum StorageConfig {
    LocalFile(local::LocalConfig),
    GCloud(gcloud::GCloudConfig),
}

#[derive(Debug, Clone)]
pub struct LocalWebServing {
    pub local_path: String,
    pub web_path: String,
}

#[async_trait]
pub trait StorageProvider: Send + Sync {
    fn local_web_serving(&self) -> Option<LocalWebServing> {
        None
    }
    async fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageIdentity>;
    async fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageIdentity>;
}

pub trait StorageUrlMaker {
    fn make_url(&self, id: &StorageIdentity) -> anyhow::Result<String>;
}

pub async fn make_provider(config: StorageConfig) -> Box<dyn StorageProvider> {
    match config {
        StorageConfig::LocalFile(local) => {
            // explicit type annotation is needed here because the other match arm is async
            Box::new(local::LocalStorageProvider::new(local)) as Box<dyn StorageProvider>
        }
        StorageConfig::GCloud(gcloud) => Box::new(gcloud::GCloudStorageProvider::new(gcloud).await),
    }
}

pub fn make_url_writer(config: StorageConfig) -> Box<dyn StorageUrlMaker> {
    match config {
        StorageConfig::LocalFile(local) => Box::new(local::LocalStorageUrlMaker::new(local)),
        StorageConfig::GCloud(gcloud) => Box::new(gcloud::GCloudStorageUrlMaker::new(gcloud)),
    }
}
