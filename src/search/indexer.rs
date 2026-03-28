use serde::{Deserialize, Serialize};

use crate::api;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexUpsertResult {
    pub datapoint_id: String,
    pub embedding_dimensions: usize,
}

#[async_trait::async_trait]
pub trait Indexer: Send + Sync {
    async fn index_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<SearchIndexUpsertResult>;
}
