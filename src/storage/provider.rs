use std::path::PathBuf;

use async_trait::async_trait;

use super::*;

#[derive(Debug, Clone)]
pub enum StorageConfig {
    LocalFile(local::LocalConfig),
    GCloud(gcloud::GCloudConfig),
}

#[async_trait]
pub trait StorageProvider: Send + Sync {
    fn local_web_serving(&self) -> Option<LocalWebServing> {
        None
    }
    async fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageIdentity>;
    async fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageIdentity>;
}

dyn_clone::clone_trait_object!(StorageProvider);

#[derive(Debug, Clone)]
pub struct LocalWebServing {
    pub local_path: String,
    pub web_path: String,
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
