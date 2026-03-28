use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    async fn embed_capture(&self, capture: &api::CaptureInfo) -> anyhow::Result<CaptureEmbedding>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureEmbedding {
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
    pub datapoint_id: String,
    pub embedding: Vec<f32>,
}
