use std::path::PathBuf;

pub mod local;
pub mod s3;

pub type StorageId = String;

#[derive(Debug, Clone)]
pub enum StorageConfig {
    Local { config: local::LocalStorageConfig },
    S3 { config: s3::S3StorageConfig },
}

pub trait StorageProvider: Send + Sync {
    fn local_serving_path(&self) -> Option<String> {
        None
    }
    fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageId>;
    fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageId>;
    fn make_url_for_id(&self, id: &StorageId) -> anyhow::Result<String>;
}

pub fn make_storage(config: StorageConfig) -> Box<dyn StorageProvider> {
    match &config {
        StorageConfig::Local { config } => {
            Box::new(local::LocalStorageProvider::new(config.clone()))
        }
        StorageConfig::S3 { config } => Box::new(s3::S3StorageProvider::new(config.clone())),
    }
}
