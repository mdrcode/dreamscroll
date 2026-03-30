use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed_capture(&self, capture: &api::CaptureInfo) -> anyhow::Result<CaptureEmbedding>;
    async fn embed_query(&self, query: &str) -> anyhow::Result<QueryEmbedding>;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CaptureEmbedding {
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
    pub illumination_text: String,
    pub embedding: Vec<f32>,
}

impl std::fmt::Debug for CaptureEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureEmbedding")
            .field("user_id", &self.user_id)
            .field("capture_id", &self.capture_id)
            .field("illumination_id", &self.illumination_id)
            .field("embedding_dims", &self.embedding.len())
            .field("illumination_text_len", &self.illumination_text.len())
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct QueryEmbedding {
    pub text: String,
    pub embedding: Vec<f32>,
}

impl std::fmt::Debug for QueryEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryEmbedding")
            .field("text_len", &self.text.len())
            .field("embedding_dims", &self.embedding.len())
            .finish()
    }
}
