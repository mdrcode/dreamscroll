use crate::search;

#[async_trait::async_trait]
pub trait EmbedPartsMaker<D: search::DataObject>: Send + Sync {
    async fn make_embed_parts(&self, object: &D) -> anyhow::Result<serde_json::Value>;
}