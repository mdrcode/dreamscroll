use anyhow::Context;
use std::collections::HashSet;

use crate::{
    api, auth, facility,
    search::{self, *},
    storage,
};

#[derive(Clone)]
pub struct CaptureSearcher {
    embedder: search::gcloud::GeminiEmbedder,
    searcher: search::gcloud::VertexAiSearcher,
    vector_store: search::gcloud::VertexAiVectorStore,
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

        let vector_store = match search::gcloud::VertexAiVectorStore::from_config(config).await {
            Ok(vector_store) => vector_store,
            Err(err) => {
                tracing::warn!(error = %err, "VertexAiVectorStore init failed; capture similarity unavailable");
                return None;
            }
        };

        Some(Self {
            embedder,
            searcher,
            vector_store,
        })
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
            let (user_id, capture_id) = match parse_fields(&hit.object_id) {
                Ok(ids) => ids,
                Err(err) => {
                    tracing::warn!(id = hit.object_id, error = %err, "Failed parsing ids from object_id; dropping hit");
                    continue;
                }
            };

            if user_id != user_context.user_id() {
                tracing::warn!(
                    hit_user_id = user_id,
                    expected_user_id = user_context.user_id(),
                    capture_id,
                    "Dropping search hit from mismatched user"
                );
                continue;
            }
            if seen_capture_ids.insert(capture_id) {
                capture_ids.push(capture_id);
            }
        }

        Ok(capture_ids)
    }

    pub async fn search_similar(
        &self,
        user_context: &auth::Context,
        query_capture: &api::CaptureInfo,
        limit: Option<u64>,
    ) -> anyhow::Result<Vec<i32>, api::ApiError> {
        let limit = limit.unwrap_or(100).clamp(1, 1000) as u32;
        let object_id = query_capture.data_object_id();

        let query_embedding = self
            .vector_store
            .fetch_object_embedding(&object_id)
            .await
            .map_err(api::ApiError::internal)?;

        let page = self
            .searcher
            .search_embedding(
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
            let (user_id, capture_id) = match parse_fields(&hit.object_id) {
                Ok(ids) => ids,
                Err(err) => {
                    tracing::warn!(id = hit.object_id, error = %err, "Failed parsing ids from object_id; dropping hit");
                    continue;
                }
            };

            if user_id != user_context.user_id() {
                tracing::warn!(
                    hit_user_id = user_id,
                    expected_user_id = user_context.user_id(),
                    capture_id = capture_id,
                    "Dropping search hit from mismatched user"
                );
                continue;
            }
            if capture_id == query_capture.id {
                continue;
            }
            if seen_capture_ids.insert(capture_id) {
                capture_ids.push(capture_id);
            }
        }

        Ok(capture_ids)
    }
}

pub(crate) fn parse_fields(doc_id: &str) -> anyhow::Result<(i32, i32)> {
    // Expected format from vector upsert path: u<user_id>-c<capture_id>
    let mut parts = doc_id.split('-');
    let user = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("data_object_id missing user"))?;
    let capture = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("data_object_id missing capture"))?;

    if parts.next().is_some() {
        anyhow::bail!("unexpected extra doc_id segments");
    }

    let user_id = user
        .strip_prefix('u')
        .ok_or_else(|| anyhow::anyhow!("data_object_id user missing 'u' prefix"))?
        .parse::<i32>()
        .context("user id is not an integer")?;
    let capture_id = capture
        .strip_prefix('c')
        .ok_or_else(|| anyhow::anyhow!("data_object_id capture missing 'c' prefix"))?
        .parse::<i32>()
        .context("capture id is not an integer")?;

    Ok((user_id, capture_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_doc_id_extracts_user_capture() {
        assert_eq!(parse_fields("u1-c123").ok(), Some((1, 123)));
        assert!(parse_fields("u1-cabc").is_err());
        assert!(parse_fields("ufoo-c1").is_err());
        assert!(parse_fields("bad").is_err());
    }
}
