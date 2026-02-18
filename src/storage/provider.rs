use std::path::PathBuf;

use async_trait::async_trait;
use dyn_clone::DynClone;

use crate::facility;

use super::*;

#[async_trait]
pub trait StorageProvider: DynClone + Send + Sync {
    async fn store_bytes(
        &self,
        bytes: &[u8],
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle>;

    async fn store_from_local_path(
        &self,
        path: &PathBuf,
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle>;

    async fn retrieve_bytes(&self, id: &StorageHandle) -> anyhow::Result<Vec<u8>>;
}

dyn_clone::clone_trait_object!(StorageProvider);

pub async fn make_provider(config: &facility::Config) -> Box<dyn StorageProvider> {
    let provider = match config.storage_backend {
        StorageBackend::Local => {
            let local_file_path = config
                .storage_local_file_path
                .as_ref()
                .expect("Storage backend is local but no file path configured");
            let local = local::LocalStorageProvider::new(local_file_path.clone());
            // explicit type annotation is needed here because the other match arm is async
            Box::new(local) as Box<dyn StorageProvider>
        }

        StorageBackend::GCloud => {
            let bucket_name = config
                .storage_gcloud_bucket_name
                .as_ref()
                .expect("Storage backend is gcloud but no bucket name configured");
            let gcloud = gcloud::GCloudStorageProvider::new(
                config.storage_gcloud_emulator_endpoint.clone(),
                bucket_name.clone(),
            )
            .await;
            Box::new(gcloud) as Box<dyn StorageProvider>
        }
    };

    tracing::info!(
        "Successfully created storage provider: {:?}",
        config.storage_backend
    );

    provider
}
