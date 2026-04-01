use anyhow;

use crate::storage;

#[async_trait::async_trait]
pub trait DataObject: Send + Sync {
    fn data_object_id(&self) -> String;

    fn data_object_json(&self) -> anyhow::Result<serde_json::Map<String, serde_json::Value>>;

    async fn parts_for_embed(
        &self,
        storage: &dyn storage::StorageProvider,
    ) -> anyhow::Result<serde_json::Value>;
}
