use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectSearchService,
    model::{
        EmbeddingTaskType, OutputFields, SearchDataObjectsResponse, SemanticSearch, TextSearch,
    },
};
use serde_json::{Value, json};

use crate::{facility, search};

use super::*;

/// Search client for Vertex Vector Search Collections.
///
/// Vertex exposes several related search "types" through
/// `DataObjectSearchService`:
///
/// 1. `search_data_objects` (single search request), where `search_type` is a
///    oneof and exactly one of these can be set:
///    - `VectorSearch`: nearest-neighbor search with a caller-provided dense or
///      sparse query vector.
///    - `SemanticSearch`: query-text search where Vertex embeds the text first
///      (requires `task_type`, e.g. `RETRIEVAL_QUERY`) and then performs vector
///      retrieval against the target vector field.
///    - `TextSearch`: lexical/keyword search over configured data fields.
///
/// 2. `batch_search_data_objects` (multiple searches in one request), which
///    runs several `Search` entries in parallel and can optionally combine
///    them with a ranker (for example Reciprocal Rank Fusion) for hybrid
///    retrieval.
///
/// 3. `query_data_objects` (filter query), which is not a rank-based search;
///    it returns objects matching a metadata/data filter and is useful for
///    lookup/browse flows.
///
/// This type currently executes `TextSearch` as an interim production-safe mode
/// for query text. The `SemanticSearch` builder is intentionally kept in place
/// so we can switch back when semantic search is configured correctly.
///
/// If we later introduce query-time embedding in app code, this same client can
/// support `VectorSearch` (or batch hybrid search) without changing the public
/// `search::Searcher` trait.
#[derive(Clone)]
pub struct VertexAiSearcher {
    collection_full_path: String,
    data_object_search_client: DataObjectSearchService,
}

impl VertexAiSearcher {
    pub async fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        let collection_id = config
            .search_embed_collection_id
            .as_ref()
            .context("SEARCH_EMBED_COLLECTION_ID required for vector search")?
            .to_string();

        Self::new(
            config.gcloud_project_id.clone(),
            config.gcloud_project_region.clone(),
            collection_id,
        )
        .await
    }

    pub async fn new(
        project_id: String,
        region: String,
        collection_id: String,
    ) -> anyhow::Result<Self> {
        let collection_full_name = format!(
            "projects/{}/locations/{}/collections/{}",
            project_id, region, collection_id
        );

        let data_object_search_client =
            DataObjectSearchService::builder()
                .build()
                .await
                .map_err(|err| {
                    anyhow::anyhow!("Failed to create DataObjectSearchService client: {}", err)
                })?;

        tracing::info!(collection_full_name, "VertexAiSearcher initialized");

        Ok(Self {
            collection_full_path: collection_full_name,
            data_object_search_client,
        })
    }

    #[allow(dead_code)]
    fn build_semantic_search(query: &search::SearchQuery) -> SemanticSearch {
        let top_k = query.limit.clamp(1, i32::MAX as u32) as i32;

        SemanticSearch::new()
            .set_search_text(query.text.clone())
            .set_search_field(constants::VECTOR_FIELD_NAME)
            .set_task_type(EmbeddingTaskType::RetrievalQuery)
            .set_top_k(top_k)
            .set_filter(
                json!({
                    "user_id": {
                        "$eq": query.user_id.to_string()
                    }
                })
                .as_object()
                .cloned()
                .expect("json object"),
            )
            .set_output_fields(OutputFields::new().set_data_fields([
                "user_id",
                "capture_id",
                "illumination_id",
            ]))
    }

    fn build_text_search(query: &search::SearchQuery) -> TextSearch {
        let top_k = query.limit.clamp(1, i32::MAX as u32) as i32;

        TextSearch::new()
            .set_search_text(query.text.clone())
            .set_top_k(top_k)
            .set_data_field_names(["illumination_text"])
            .set_output_fields(OutputFields::new().set_data_fields([
                "user_id",
                "capture_id",
                "illumination_id",
            ]))
    }

    fn map_search_data_objects_response(
        response: SearchDataObjectsResponse,
    ) -> search::SearchResultPage {
        tracing::info!(
            num_hits = response.results.len(),
            next_page_token = response.next_page_token,
            "Vertex search returned results"
        );

        let hits = response
            .results
            .into_iter()
            .filter_map(|result| {
                let Some(data_object) = result.data_object else {
                    tracing::warn!("Search result missing data_object; dropping hit");
                    return None;
                };

                let Some(data) = data_object.data else {
                    tracing::warn!("Search result data_object missing data field; dropping");
                    return None;
                };

                let doc_id = data_object.data_object_id;
                let Some(capture_id) = parse_i32_field(&data, "capture_id") else {
                    tracing::warn!(doc_id, "Search result missing capture_id; dropping");
                    return None;
                };
                let Some(illumination_id) = parse_i32_field(&data, "illumination_id") else {
                    tracing::warn!(doc_id, "Search result missing illumination_id; dropping");
                    return None;
                };
                let Some(score) = result.distance else {
                    tracing::warn!(doc_id, "Search result missing distance score; dropping");
                    return None;
                };

                Some(search::SearchHit {
                    corpus_doc_id: doc_id,
                    capture_id,
                    illumination_id,
                    score,
                })
            })
            .collect();

        search::SearchResultPage {
            hits,
            next_page_token: if response.next_page_token.is_empty() {
                None
            } else {
                Some(response.next_page_token)
            },
        }
    }
}

#[async_trait::async_trait]
impl search::Searcher for VertexAiSearcher {
    #[tracing::instrument(skip(self, query), fields(user_id = query.user_id, limit = query.limit))]
    async fn search(
        &self,
        query: &search::SearchQuery,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query.text.trim().is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        let text_search = Self::build_text_search(query);
        tracing::info!("Built TextSearch request: {:?}", text_search);

        let mut req = self
            .data_object_search_client
            .search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_text_search(text_search);

        if let Some(page_token) = query.page_token.as_ref() {
            req = req.set_page_token(page_token.clone());
        }

        let response = match req.send().await {
            Ok(response) => response,
            Err(err) => {
                let status_code = err.status().map(|s| s.code as i32);
                let http_status = err.http_status_code();
                tracing::error!(
                    error = %err,
                    status_code,
                    http_status,
                    collection = self.collection_full_path,
                    user_id = query.user_id,
                    query_text = query.text,
                    "Vertex text search failed"
                );
                anyhow::bail!(
                    "Vertex text search failed: {} (status_code={:?}, http_status={:?})",
                    err,
                    status_code,
                    http_status
                );
            }
        };
        Ok(Self::map_search_data_objects_response(response))
    }
}

fn parse_i32_field(data: &serde_json::Map<String, Value>, key: &str) -> Option<i32> {
    let raw = data.get(key)?;

    if let Some(number) = raw.as_i64() {
        return i32::try_from(number).ok();
    }

    raw.as_str()?.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_i32_field_accepts_number_and_string() {
        let value = json!({
            "capture_id": 123,
            "illumination_id": "456"
        });

        let obj = value.as_object().expect("json object");
        assert_eq!(parse_i32_field(obj, "capture_id"), Some(123));
        assert_eq!(parse_i32_field(obj, "illumination_id"), Some(456));
        assert_eq!(parse_i32_field(obj, "missing"), None);
    }
}
