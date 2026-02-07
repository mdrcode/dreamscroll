use std::path::{Path, PathBuf};

use async_trait::async_trait;
use uuid::Uuid;

use super::*;

#[derive(Clone)]
pub struct LocalStorageProvider {
    path: String,
}

impl LocalStorageProvider {
    pub fn new(local_path: String) -> Self {
        Self { path: local_path }

        // TODO check if the path exists and is writable?
    }
}

#[async_trait]
impl StorageProvider for LocalStorageProvider {
    async fn store_from_bytes(&self, bytes: &[u8]) -> anyhow::Result<StorageIdentity> {
        let uuid = Uuid::new_v4().to_string();
        let upload_path = Path::new(&self.path).join(uuid.as_str());
        tokio::fs::write(&upload_path, &bytes).await?;
        Ok(StorageIdentity {
            storage_provider: "local".to_string(),
            provider_id: uuid,
            provider_shard: None,
            provider_bucket: None,
        })
    }

    async fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageIdentity> {
        let uuid = Uuid::new_v4().to_string();
        let upload_path = Path::new(&self.path).join(uuid.as_str());
        tracing::warn!(
            "Storing from local path {:?} to local storage {}.",
            path,
            upload_path.display()
        );
        tokio::fs::copy(path, &upload_path).await?;

        Ok(StorageIdentity {
            storage_provider: "local".to_string(),
            provider_id: uuid,
            provider_shard: None,
            provider_bucket: None,
        })
    }

    async fn retrieve_bytes(&self, id: &StorageIdentity) -> anyhow::Result<Vec<u8>> {
        let file_path = Path::new(&self.path).join(&id.provider_id);
        let bytes = tokio::fs::read(&file_path).await?;
        Ok(bytes)
    }
}
