use serde::{Deserialize, Serialize};

use super::CaptureEmbedding;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexUpsertResult {
    pub datapoint_id: String,
    pub embedding_dimensions: usize,
}

#[async_trait::async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert_embedding(
        &self,
        embedded: &CaptureEmbedding,
    ) -> anyhow::Result<SearchIndexUpsertResult>;
}
