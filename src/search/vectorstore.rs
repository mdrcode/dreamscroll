use serde::{Deserialize, Serialize};

use super::*;

#[async_trait::async_trait]
pub trait VectorStore: Send + Sync {
    async fn upsert_capture_embedding(
        &self,
        embedding: &CaptureEmbedding,
    ) -> anyhow::Result<VectorUpsertResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorUpsertResult {
    pub id: String,
    pub fq_id: Option<String>,
    pub dims: usize,
}
