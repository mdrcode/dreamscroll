use std::path::PathBuf;

pub mod local;
pub mod s3;

pub type StorageId = String;

#[derive(Debug, Clone)]
pub enum StorageConfig {
    Local {
        storage_path: String,
        base_url: String,
    },
    S3 {
        bucket: String,
        region: String,
        access_key: String,
        secret_key: String,
        base_url: String,
    },
}

pub trait StorageProvider: Send + Sync {
    fn local_serving_path(&self) -> Option<String> {
        None
    }
    fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageId>;
    fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageId>;
    fn make_url_for_id(&self, id: &StorageId) -> anyhow::Result<String>;
}

pub fn make(config: StorageConfig) -> Box<dyn StorageProvider> {
    match &config {
        StorageConfig::Local {
            storage_path,
            base_url: _,
        } => Box::new(local::LocalStorageProvider::new(storage_path.clone())),

        StorageConfig::S3 {
            bucket: _,
            region: _,
            access_key: _,
            secret_key: _,
            base_url: _,
        } => unimplemented!(),
    }
}
