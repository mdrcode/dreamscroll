use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectSearchService,
    model::{
        DenseVector, Ranker, ReciprocalRankFusion, Search, SearchDataObjectsResponse, TextSearch,
        VectorSearch, batch_search_data_objects_request::CombineResultsOptions,
    },
};
use serde_json::json;

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
/// VertextAiSearcher composes the first two primitives to implement the
/// search::Searcher trait for text, vector, and hybrid search modes.
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

    fn query_top_k(limit: u32) -> i32 {
        limit.clamp(1, 1000) as i32
    }

    fn make_user_filter(user_id: i32) -> serde_json::Map<String, serde_json::Value> {
        json!({
            "user_id": {
                "$eq": user_id.to_string()
            }
        })
        .as_object()
        .cloned()
        .expect("json object")
    }

    fn make_text_search(query_text: &str, params: &search::QueryParams) -> TextSearch {
        TextSearch::new()
            .set_search_text(query_text)
            .set_data_field_names(["illumination_text"])
            .set_top_k(Self::query_top_k(params.limit))
            .set_filter(Self::make_user_filter(params.user_id))
    }

    fn make_vector_search(
        query_embed: &search::Embedding<f32, search::Unit>,
        params: &search::QueryParams,
    ) -> VectorSearch {
        VectorSearch::new()
            .set_search_field(constants::CAPTURE_DENSE_VECTOR)
            .set_vector(DenseVector::new().set_values(query_embed.as_slice().to_vec()))
            .set_top_k(Self::query_top_k(params.limit))
            .set_filter(Self::make_user_filter(params.user_id))
    }
}

#[async_trait::async_trait]
impl search::Searcher<search::Embedding<f32, search::Unit>> for VertexAiSearcher {
    #[tracing::instrument(skip(self, params), fields(user_id = params.user_id, limit = params.limit, query_len = query_text.len()))]
    async fn search_text(
        &self,
        query_text: &str,
        params: &search::QueryParams,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query_text.trim().is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        let mut request = self
            .data_object_search_client
            .search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_text_search(Self::make_text_search(query_text, params));

        if let Some(page_token) = params.page_token.as_ref() {
            request = request.set_page_token(page_token.clone());
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                let status_code = err.status().map(|s| s.code as i32);
                let http_status = err.http_status_code();
                tracing::error!(
                    collection = self.collection_full_path,
                    user_id = params.user_id,
                    status_code,
                    http_status,
                    error = ?err,
                    "Vertex text search failed"
                );
                anyhow::bail!(
                    "Vertex text search failed status_code={:?} http_status={:?}",
                    status_code,
                    http_status,
                );
            }
        };

        tracing::debug!(
            num_hits = response.results.len(),
            next_page_token = response.next_page_token,
            "Vertex text search returned results"
        );

        Ok(map_search_data_objects_response(response))
    }

    #[tracing::instrument(skip(self, query_embed, params), fields(user_id = params.user_id, limit = params.limit, dims = query_embed.len()))]
    async fn search_embedding(
        &self,
        query_embed: &search::Embedding<f32, search::Unit>,
        params: &search::QueryParams,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query_embed.is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        let mut request = self
            .data_object_search_client
            .search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_vector_search(Self::make_vector_search(query_embed, params));

        if let Some(page_token) = params.page_token.as_ref() {
            request = request.set_page_token(page_token.clone());
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                let status_code = err.status().map(|s| s.code as i32);
                let http_status = err.http_status_code();
                tracing::error!(
                    collection = self.collection_full_path,
                    user_id = params.user_id,
                    dims = query_embed.len(),
                    status_code,
                    http_status,
                    error = ?err,
                    "Vertex vector search failed"
                );
                anyhow::bail!(
                    "Vertex vector search failed status_code={:?} http_status={:?}",
                    status_code,
                    http_status,
                );
            }
        };

        tracing::debug!(
            num_hits = response.results.len(),
            next_page_token = response.next_page_token,
            "Vertex search returned results"
        );

        Ok(map_search_data_objects_response(response))
    }

    #[tracing::instrument(skip(self, query_embed, params), fields(user_id = params.user_id, limit = params.limit, dims = query_embed.len(), query_len = query_text.len()))]
    async fn search_hybrid(
        &self,
        query_text: &str,
        query_embed: &search::Embedding<f32, search::Unit>,
        params: &search::QueryParams,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query_embed.is_empty() || query_text.trim().is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        if params.page_token.is_some() {
            tracing::warn!("Hybrid batch search currently ignores page_token");
        }

        let top_k = Self::query_top_k(params.limit);

        let vector_search = Self::make_vector_search(query_embed, params);

        let text_search = Self::make_text_search(query_text, params);

        let combine = CombineResultsOptions::new()
            .set_ranker(Ranker::new().set_rrf(ReciprocalRankFusion::new().set_weights([1.0, 1.0])))
            .set_top_k(top_k);

        let response = match self
            .data_object_search_client
            .batch_search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_searches([
                Search::new().set_vector_search(vector_search),
                Search::new().set_text_search(text_search),
            ])
            .set_combine(combine)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                let status_code = err.status().map(|s| s.code as i32);
                let http_status = err.http_status_code();
                tracing::error!(
                    collection = self.collection_full_path,
                    user_id = params.user_id,
                    dims = query_embed.len(),
                    status_code,
                    http_status,
                    error = ?err,
                    "Vertex hybrid search failed"
                );
                anyhow::bail!(
                    "Vertex hybrid search failed status_code={:?} http_status={:?}",
                    status_code,
                    http_status,
                );
            }
        };

        let mut result_pages = response.results.into_iter();
        let Some(combined_page) = result_pages.next() else {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        };

        if result_pages.next().is_some() {
            tracing::warn!(
                "Hybrid search returned multiple pages despite combine options; using first page"
            );
        }

        Ok(map_search_data_objects_response(combined_page))
    }
}

fn map_search_data_objects_response(
    response: SearchDataObjectsResponse,
) -> search::SearchResultPage {
    let hits = response
        .results
        .into_iter()
        .filter_map(|result| {
            let Some(data_object) = result.data_object else {
                tracing::warn!("Search result missing data_object; dropping hit");
                return None;
            };
            let doc_id = data_object.data_object_id;

            let (user_id, capture_id, illumination_id) = match data_object_id::parse_fields(&doc_id)
            {
                Ok(ids) => ids,
                Err(err) => {
                    tracing::warn!(doc_id, error = %err, "Failed parsing ids from doc_id; dropping hit");
                    return None;
                }
            };
            let Some(score) = result.distance else {
                tracing::warn!(doc_id, "Search result missing distance score; dropping");
                return None;
            };

            Some(search::SearchHit {
                doc_id,
                user_id,
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
