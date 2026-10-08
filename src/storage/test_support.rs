use async_trait::async_trait;
use bytes::Bytes;

use super::{StorageHandle, StorageProvider};

#[derive(Clone)]
pub(crate) struct TestStorageProvider;

#[async_trait]
impl StorageProvider for TestStorageProvider {
    async fn store_bytes(
        &self,
        _bytes: Bytes,
        _user_shard: &str,
        _ext: Option<&str>,
        _mime_type: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        anyhow::bail!("unused in illumination worker test")
    }

    async fn store_from_local_path(
        &self,
        _path: &std::path::Path,
        _user_shard: &str,
        _ext: Option<&str>,
        _mime_type: Option<&str>,
    ) -> anyhow::Result<StorageHandle> {
        anyhow::bail!("unused in illumination worker test")
    }

    async fn retrieve_bytes(&self, _handle: &StorageHandle) -> anyhow::Result<Vec<u8>> {
        Ok(vec![1, 2, 3])
    }

    fn make_prod_uri(&self, _handle: &StorageHandle) -> anyhow::Result<String> {
        anyhow::bail!("unused in illumination worker test")
    }
}
