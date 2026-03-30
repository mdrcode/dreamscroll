use anyhow::Context;
use google_cloud_vectorsearch_v1::{
    client::DataObjectSearchService,
    model::{DenseVector, SearchDataObjectsResponse, VectorSearch},
};

use crate::search::gcloud::constants;
use crate::{facility, search};

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

    fn build_vector_search(query: &search::SearchQueryEmbedding) -> VectorSearch {
        let top_k = query.limit.clamp(1, i32::MAX as u32) as i32;

        // Intentionally omit output_fields while debugging INVALID_ARGUMENT behavior.
        VectorSearch::new()
            .set_search_field(constants::VECTOR_FIELD_NAME)
            .set_vector(DenseVector::new().set_values(query.query_embedding.clone()))
            .set_top_k(top_k)
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
                let doc_id = data_object.data_object_id;

                let Some((capture_id, illumination_id)) =
                    parse_capture_and_illumination_from_doc_id(&doc_id)
                else {
                    tracing::warn!(
                        doc_id,
                        "Search result doc_id format unexpected; dropping hit"
                    );
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
    #[tracing::instrument(skip(self, query), fields(user_id = query.user_id, limit = query.limit, dims = query.query_embedding.len()))]
    async fn search_query_embedding(
        &self,
        query: &search::SearchQueryEmbedding,
    ) -> anyhow::Result<search::SearchResultPage> {
        if query.query_embedding.is_empty() {
            return Ok(search::SearchResultPage {
                hits: vec![],
                next_page_token: None,
            });
        }

        let vector_search = Self::build_vector_search(query);

        let mut req = self
            .data_object_search_client
            .search_data_objects()
            .set_parent(self.collection_full_path.clone())
            .set_vector_search(vector_search);

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
                    dims = query.query_embedding.len(),
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
        Ok(Self::map_search_data_objects_response(response))
    }
}

fn parse_capture_and_illumination_from_doc_id(doc_id: &str) -> Option<(i32, i32)> {
    // Expected format from vector upsert path: u<user_id>-c<capture_id>-i<illumination_id>
    let mut parts = doc_id.split('-');
    let _user = parts.next()?;
    let capture = parts.next()?;
    let illumination = parts.next()?;

    if parts.next().is_some() {
        return None;
    }

    let capture_id = capture.strip_prefix('c')?.parse::<i32>().ok()?;
    let illumination_id = illumination.strip_prefix('i')?.parse::<i32>().ok()?;
    Some((capture_id, illumination_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_doc_id_extracts_capture_and_illumination() {
        assert_eq!(
            parse_capture_and_illumination_from_doc_id("u986ewyyn-c123-i456"),
            Some((123, 456))
        );
        assert_eq!(
            parse_capture_and_illumination_from_doc_id("u1-cabc-i2"),
            None
        );
        assert_eq!(parse_capture_and_illumination_from_doc_id("bad"), None);
    }
}
