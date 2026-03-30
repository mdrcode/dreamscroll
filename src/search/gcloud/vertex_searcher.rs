use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectSearchService,
    model::{DenseVector, SearchDataObjectsResponse, VectorSearch},
};

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
/// This type currently executes explicit `VectorSearch` using caller-provided
/// query embeddings.
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
}

#[async_trait::async_trait]
impl search::Searcher for VertexAiSearcher {
    #[tracing::instrument(skip(self, query_embed, params), fields(user_id = params.user_id, limit = params.limit, dims = query_embed.embedding.len()))]
    async fn search_query_embedding(
        &self,
        query_embed: &search::QueryEmbedding,
        params: &search::QueryParams,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query_embed.embedding.is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        // BUG? Every attempt to use set_output_fields results in a 400 Bad
        // Request with message "invalid argument". For now, we simply encode
        // the fields we need in the object id and extract them at search time.
        // See data_object_id.rs
        //
        // Similarly, every attempt to filter on user_id result in zero search
        // hits, even though user_id is present in the indexed objects.
        //
        // Temporary trust boundary: this stage may return cross-tenant candidate
        // ids, and authorization is enforced by the capture fetch API that
        // filters by user_id.
        let vector_search = VectorSearch::new()
            .set_search_field(constants::CAPTURE_DENSE_VECTOR)
            .set_vector(DenseVector::new().set_values(query_embed.embedding.clone()))
            .set_top_k(params.limit.clamp(1, 1000) as i32);
        // .set_output_fields(OutputFields::new().set_data_fields(["user_id"]))
            // .set_filter(
            //     json!({
            //         "user_id": {
            //             "$eq": params.user_id.to_string()
            //         }
            //     })
            //     .as_object()
            //     .cloned()
            //     .expect("json object"),
            // )

        let mut request = self
            .data_object_search_client
            .search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_vector_search(vector_search);

        if let Some(page_token) = params.page_token.as_ref() {
            request = request.set_page_token(page_token.clone());
        }

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                let status_code = err.status().map(|s| s.code as i32);
                let http_status = err.http_status_code();
                tracing::error!(
                    error = %err,
                    status_code,
                    http_status,
                    collection = self.collection_full_path,
                    user_id = params.user_id,
                    dims = query_embed.embedding.len(),
                    "Vertex vector search failed"
                );
                anyhow::bail!(
                    "Vertex vector search failed: {} (status_code={:?}, http_status={:?})",
                    err,
                    status_code,
                    http_status
                );
            }
        };

        tracing::info!(
            num_hits = response.results.len(),
            next_page_token = response.next_page_token,
            "Vertex search returned results"
        );

        Ok(map_search_data_objects_response(response))
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
