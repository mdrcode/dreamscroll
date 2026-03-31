use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Embedder<E>: Send + Sync {
    async fn embed_query(&self, query: &str) -> anyhow::Result<E>;

    async fn embed_capture(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<CaptureEmbedding<E>>;
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CaptureEmbedding<E> {
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
    pub illumination_text: String,
    pub embedding: E,
}

impl<E: std::fmt::Debug> std::fmt::Debug for CaptureEmbedding<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureEmbedding")
            .field("user_id", &self.user_id)
            .field("capture_id", &self.capture_id)
            .field("illumination_id", &self.illumination_id)
            .field("embedding", &self.embedding)
            .field("illumination_text_len", &self.illumination_text.len())
            .finish()
    }
}
