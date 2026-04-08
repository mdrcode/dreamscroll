use crate::{api, storage};

use super::*;

#[derive(Clone)]
pub struct CaptureEmbedder {
    pub embedder: gcloud::GeminiEmbedder<api::CaptureInfo, CaptureInfoEmbedPartsMaker>,
    pub vector_store: gcloud::VertexVectorStore,
}

impl CaptureEmbedder {
    pub async fn from_config(
        config: &crate::facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Option<Self> {
        let parts_maker = CaptureInfoEmbedPartsMaker::new(storage);
        let embedder = match gcloud::GeminiEmbedder::from_config(config, parts_maker) {
            Ok(embedder) => embedder,
            Err(err) => {
                tracing::warn!(error = %err, "GeminiEmbedder init failed for CaptureEmbedder");
                return None;
            }
        };

        let vector_store = match gcloud::VertexVectorStore::from_config(config).await {
            Ok(vector_store) => vector_store,
            Err(err) => {
                tracing::warn!(error = %err, "VertexVectorStore init failed for CaptureEmbedder");
                return None;
            }
        };

        Some(Self {
            embedder,
            vector_store,
        })
    }
}
