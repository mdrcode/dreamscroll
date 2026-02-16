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
    async fn store_bytes(
        &self,
        bytes: &[u8],
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        let shard_dir = Path::new(&self.path).join(user_shard);
        tokio::fs::create_dir_all(&shard_dir).await?;

        let uuid = Uuid::new_v4();
        let file_path = shard_dir
            .join(uuid.to_string())
            .with_extension(ext.unwrap_or(""));
        tokio::fs::write(&file_path, &bytes).await?;

        Ok(StorageHandle {
            provider: "local".to_string(),
            uuid,
            user_shard: user_shard.to_string(),
            bucket: None,
            extension: ext.map(|s| s.to_string()),
        })
    }

    async fn store_from_local_path(
        &self,
        source_path: &PathBuf,
        user_shard: &str,
        ext: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        let shard_dir = Path::new(&self.path).join(user_shard);
        tokio::fs::create_dir_all(&shard_dir).await?;

        let uuid = Uuid::new_v4();
        let file_path = shard_dir
            .join(uuid.to_string())
            .with_extension(ext.unwrap_or(""));
        tracing::info!(
            "Storing from local path {:?} to local storage {}.",
            source_path,
            file_path.display()
        );
        tokio::fs::copy(source_path, &file_path).await?;

        Ok(StorageHandle {
            provider: "local".to_string(),
            uuid,
            user_shard: user_shard.to_string(),
            bucket: None,
            extension: ext.map(|s| s.to_string()),
        })
    }

    async fn retrieve_bytes(&self, h: &StorageHandle) -> anyhow::Result<Vec<u8>> {
        let file_path = PathBuf::from(&self.path)
            .join(&h.user_shard)
            .join(h.uuid.to_string())
            .with_extension(h.extension.as_deref().unwrap_or(""));
        let bytes = tokio::fs::read(&file_path).await?;
        Ok(bytes)
    }
}
