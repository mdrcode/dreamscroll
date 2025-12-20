use std::path::PathBuf;

use super::{StorageId, StorageProvider};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct S3StorageConfig {
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    base_url: String,
}

#[allow(dead_code)]
pub struct S3StorageProvider {
    config: S3StorageConfig,
}
impl S3StorageProvider {
    pub fn new(config: S3StorageConfig) -> Self {
        Self { config }
    }
}

#[allow(unused_variables)]
impl StorageProvider for S3StorageProvider {
    fn store_from_bytes(&self, data: &[u8]) -> anyhow::Result<StorageId> {
        unimplemented!()
    }

    fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageId> {
        let bytes = std::fs::read(path)?;
        self.store_from_bytes(&bytes)
    }

    fn make_url_for_id(&self, id: &StorageId) -> anyhow::Result<String> {
        unimplemented!()
    }
}
