use crate::{api, search, storage};

use super::*;

#[derive(Clone)]
pub struct SearchIndexer {
    pub embedder: search::gcloud::GeminiEmbedder<api::CaptureInfo, CaptureInfoEmbedPartsMaker>,
    pub vector_store: search::gcloud::VertexVectorStore,
}

impl SearchIndexer {
    pub async fn from_config(
        config: &crate::facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Option<Self> {
        let parts_maker = CaptureInfoEmbedPartsMaker::new(storage);
        let embedder = match search::gcloud::GeminiEmbedder::from_config(config, parts_maker) {
            Ok(embedder) => embedder,
            Err(err) => {
                tracing::warn!(error = %err, "GeminiEmbedder init failed for SearchIndexer");
                return None;
            }
        };

        let vector_store = match search::gcloud::VertexVectorStore::from_config(config).await {
            Ok(vector_store) => vector_store,
            Err(err) => {
                tracing::warn!(error = %err, "VertexVectorStore init failed for SearchIndexer");
                return None;
            }
        };

        Some(Self {
            embedder,
            vector_store,
        })
    }
}
