use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::{StorageId, StorageProvider};

pub struct LocalStorageProvider {
    storage_path: String,
}

impl LocalStorageProvider {
    pub fn new(storage_path: String) -> Self {
        std::fs::create_dir_all(&storage_path).unwrap();
        Self { storage_path }
    }
}

impl StorageProvider for LocalStorageProvider {
    fn local_serving_path(&self) -> Option<String> {
        Some(self.storage_path.clone())
    }

    fn store_from_bytes(&self, bytes: &[u8]) -> anyhow::Result<StorageId> {
        let uuid = Uuid::new_v4().to_string();
        let upload_path = Path::new(&self.storage_path).join(uuid.as_str());
        std::fs::write(&upload_path, &bytes)?;
        Ok(uuid)
    }

    fn store_from_local_path(&self, path: &PathBuf) -> anyhow::Result<StorageId> {
        let uuid = Uuid::new_v4().to_string();
        let upload_path = Path::new(&self.storage_path).join(uuid.as_str());
        std::fs::copy(path, &upload_path)?;
        Ok(uuid)
    }

    fn make_url_for_id(&self, id: &StorageId) -> anyhow::Result<String> {
        // Implementation for generating URL for local storage
        Ok(format!("http://localhost/storage/{}", id))
    }
}
