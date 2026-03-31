use std::collections::HashSet;

use crate::search::{Embedder, Searcher};
use crate::{api, auth, facility, search, storage};

#[derive(Clone)]
pub struct CaptureSearcher {
    embedder: search::gcloud::GeminiEmbedder,
    searcher: search::gcloud::VertexAiSearcher,
}

impl CaptureSearcher {
    pub async fn from_config(
        config: &facility::Config,
        storage: Box<dyn storage::StorageProvider>,
    ) -> Option<Self> {
        let embedder = match search::gcloud::GeminiEmbedder::from_config(config, storage) {
            Ok(embedder) => embedder,
            Err(err) => {
                tracing::warn!(error = %err, "GeminiEmbedder init failed; web search unavailable");
                return None;
            }
        };

        let searcher = match search::gcloud::VertexAiSearcher::from_config(config).await {
            Ok(searcher) => searcher,
            Err(err) => {
                tracing::warn!(error = %err, "VertexAiSearcher init failed; web search unavailable");
                return None;
            }
        };

        Some(Self { embedder, searcher })
    }

    pub async fn search(
        &self,
        user_context: &auth::Context,
        query: &str,
        limit: Option<u64>,
    ) -> anyhow::Result<Vec<i32>, api::ApiError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(vec![]);
        }

        let limit = limit.unwrap_or(100).clamp(1, 1000) as u32;

        let query_embedding = self
            .embedder
            .embed_query(query)
            .await
            .map_err(api::ApiError::internal)?;

        let page = self
            .searcher
            .search_hybrid(
                query,
                &query_embedding,
                &search::QueryParams {
                    user_id: user_context.user_id(),
                    limit,
                    page_token: None,
                },
            )
            .await
            .map_err(api::ApiError::internal)?;

        let mut seen_capture_ids = HashSet::new();
        let mut capture_ids = Vec::new();
        for hit in page.hits {
            if hit.user_id != user_context.user_id() {
                tracing::warn!(
                    hit_user_id = hit.user_id,
                    expected_user_id = user_context.user_id(),
                    capture_id = hit.capture_id,
                    "Dropping search hit from mismatched user"
                );
                continue;
            }
            if seen_capture_ids.insert(hit.capture_id) {
                capture_ids.push(hit.capture_id);
            }
        }

        Ok(capture_ids)
    }
}
