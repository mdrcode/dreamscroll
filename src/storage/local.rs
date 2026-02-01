use std::path::{Path, PathBuf};

use async_trait::async_trait;
use uuid::Uuid;

use super::*;

#[derive(Debug, Clone)]
pub struct LocalConfig {
    pub storage_path: String,
    pub web_path: String,
}

pub struct LocalStorageProvider {
    config: LocalConfig,
}

impl LocalStorageProvider {
    pub fn new(config: LocalConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl StorageProvider for LocalStorageProvider {
    fn local_web_serving(&self) -> Option<LocalWebServing> {
        Some(LocalWebServing {
            local_path: self.config.storage_path.clone(),
            web_path: self.config.web_path.clone(),
        })
    }

    async fn store_from_bytes(&self, bytes: &[u8]) -> anyhow::Result<StorageIdentity> {
        let uuid = Uuid::new_v4().to_string();
        let upload_path = Path::new(&self.config.storage_path).join(uuid.as_str());
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
        let upload_path = Path::new(&self.config.storage_path).join(uuid.as_str());
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
}
