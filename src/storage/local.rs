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
    async fn store_bytes(&self, bytes: &[u8], user_shard: &str) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let shard_dir = Path::new(&self.path).join(user_shard);
        tokio::fs::create_dir_all(&shard_dir).await?;
        let upload_path = shard_dir.join(uuid.to_string());
        tokio::fs::write(&upload_path, &bytes).await?;
        Ok(StorageHandle {
            provider: "local".to_string(),
            uuid,
            user_shard: user_shard.to_string(),
            bucket: None,
        })
    }

    async fn store_from_local_path(
        &self,
        path: &PathBuf,
        user_shard: &str,
    ) -> anyhow::Result<StorageHandle> {
        let uuid = Uuid::new_v4();
        let shard_dir = Path::new(&self.path).join(user_shard);
        tokio::fs::create_dir_all(&shard_dir).await?;
        let upload_path = shard_dir.join(uuid.to_string());
        tracing::info!(
            "Storing from local path {:?} to local storage {}.",
            path,
            upload_path.display()
        );
        tokio::fs::copy(path, &upload_path).await?;

        Ok(StorageHandle {
            provider: "local".to_string(),
            uuid,
            user_shard: user_shard.to_string(),
            bucket: None,
        })
    }

    async fn retrieve_bytes(&self, id: &StorageHandle) -> anyhow::Result<Vec<u8>> {
        let file_path = PathBuf::from(&self.path)
            .join(&id.user_shard)
            .join(id.uuid.to_string());
        let bytes = tokio::fs::read(&file_path).await?;
        Ok(bytes)
    }
}
