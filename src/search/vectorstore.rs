use serde::{Deserialize, Serialize};

use super::*;

#[async_trait::async_trait]
pub trait VectorStore<E>: Send + Sync {
    async fn upsert_object_embedding(
        &self,
        object: &dyn DataObject,
        embedding: &E,
    ) -> anyhow::Result<VectorUpsertResult>;

    async fn fetch_object_embedding(&self, object_id: &str) -> anyhow::Result<Option<E>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorUpsertResult {
    pub id: String,
    pub fq_id: Option<String>,
    pub dims: usize,
}
