use std::path::PathBuf;

use async_trait::async_trait;
use dyn_clone::DynClone;

use crate::facility;

use super::*;

#[async_trait]
pub trait StorageProvider: DynClone + Send + Sync {
    async fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageIdentity>;
    async fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageIdentity>;
    async fn retrieve_bytes(&self, id: &StorageIdentity) -> anyhow::Result<Vec<u8>>;
}

dyn_clone::clone_trait_object!(StorageProvider);

pub async fn make_provider(config: &facility::Config) -> Box<dyn StorageProvider> {
    match config.storage_backend {
        StorageBackend::Local => {
            let local = local::LocalStorageProvider::new(config.storage_local_path.clone());
            // explicit type annotation is needed here because the other match arm is async
            Box::new(local) as Box<dyn StorageProvider>
        }

        StorageBackend::GCloud => {
            let gcloud = gcloud::GCloudStorageProvider::new(
                config.storage_gcloud_emulator_endpoint.clone(),
                config.storage_gcloud_bucket_name.clone(),
            )
            .await;
            Box::new(gcloud) as Box<dyn StorageProvider>
        }
    }
}
