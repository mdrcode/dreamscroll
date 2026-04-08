use crate::storage;

use super::*;

#[derive(Clone)]
pub struct CaptureEmbedder {
    // TODO revisit pub on these
    pub embed_parts_maker: CaptureInfoEmbedMaker,
    pub embedder: gcloud::GeminiEmbedder,
    pub vector_store: gcloud::VertexVectorStore,
}

impl CaptureEmbedder {
    pub async fn from_config(
        config: &crate::facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Option<Self> {
        let embed_parts_maker = CaptureInfoEmbedMaker::new(storage);
        let embedder = match gcloud::GeminiEmbedder::from_config(config) {
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
            embed_parts_maker,
            embedder,
            vector_store,
        })
    }
}
