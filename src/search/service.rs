use std::collections::HashSet;

use anyhow::Context;

use crate::{api, auth, config};

use super::gcloud::{GeminiEmbedder, VertexVectorSearcher, VertexVectorStore};
use super::local::{LocalEmbedder, LocalSearch};
use super::{Embedder, Embedding, QueryParams, SearchHit, Searcher, Unit, VectorStore};

/// Search facade and its shared provider components.
///
/// WebUI search and background search-index tasks must use the same instance so
/// a local index written by one is visible to the other; the composition root
/// shares it through an `Arc`.
pub struct SearchService {
    embedder: Box<dyn Embedder<serde_json::Value, Embedding<f32, Unit>>>,
    searcher: Box<dyn Searcher<Embedding<f32, Unit>>>,
    vector_store: Box<dyn VectorStore<Embedding<f32, Unit>>>,
}

impl SearchService {
    pub async fn from_config(cfg: &config::Config) -> anyhow::Result<Self> {
        match cfg.search_backend {
            config::SearchBackend::Gcloud => {
                let embedder = Box::new(
                    GeminiEmbedder::from_config(cfg).context("GeminiEmbedder init failed")?,
                );
                let searcher = Box::new(
                    VertexVectorSearcher::from_config(cfg)
                        .await
                        .context("VertexVectorSearcher init failed")?,
                );
                let vector_store = Box::new(
                    VertexVectorStore::from_config(cfg)
                        .await
                        .context("VertexVectorStore init failed")?,
                );
                Ok(Self {
                    embedder,
                    searcher,
                    vector_store,
                })
            }
            config::SearchBackend::Local => {
                let local = LocalSearch::default();
                Ok(Self {
                    embedder: Box::new(LocalEmbedder::new()),
                    searcher: Box::new(local.clone()),
                    vector_store: Box::new(local),
                })
            }
        }
    }

    pub fn embedder(&self) -> &dyn Embedder<serde_json::Value, Embedding<f32, Unit>> {
        self.embedder.as_ref()
    }

    pub fn vector_store(&self) -> &dyn VectorStore<Embedding<f32, Unit>> {
        self.vector_store.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        embedder: Box<dyn Embedder<serde_json::Value, Embedding<f32, Unit>>>,
        searcher: Box<dyn Searcher<Embedding<f32, Unit>>>,
        vector_store: Box<dyn VectorStore<Embedding<f32, Unit>>>,
    ) -> Self {
        Self {
            embedder,
            searcher,
            vector_store,
        }
    }

    fn user_filter(user_id: i32) -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({
            "user_id": {
                "$eq": user_id.to_string()
            }
        })
        .as_object()
        .cloned()
        .expect("json object")
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
                &QueryParams {
                    base_filter: Self::user_filter(user_context.user_id()),
                    limit,
                    page_token: None,
                },
            )
            .await
            .map_err(api::ApiError::internal)?;

        Ok(capture_ids_from_hits(
            page.hits,
            user_context.user_id(),
            None,
        ))
    }

    pub async fn search_similar(
        &self,
        user_context: &auth::Context,
        query_capture_id: i32,
        limit: Option<u64>,
    ) -> anyhow::Result<Vec<i32>, api::ApiError> {
        let limit = limit.unwrap_or(100).clamp(1, 1000) as u32;
        let object_id = format!("u{}-c{}", user_context.user_id(), query_capture_id);

        let query_embedding = self
            .vector_store
            .fetch_object_embedding(&object_id)
            .await
            .map_err(api::ApiError::internal)?;
        let Some(query_embedding) = query_embedding else {
            tracing::warn!(
                capture_id = query_capture_id,
                object_id,
                "No embedding found for query capture; returning empty similar results"
            );
            return Ok(vec![]);
        };

        let page = self
            .searcher
            .search_embedding(
                &query_embedding,
                &QueryParams {
                    base_filter: Self::user_filter(user_context.user_id()),
                    limit,
                    page_token: None,
                },
            )
            .await
            .map_err(api::ApiError::internal)?;

        Ok(capture_ids_from_hits(
            page.hits,
            user_context.user_id(),
            Some(query_capture_id),
        ))
    }
}

fn capture_ids_from_hits(
    hits: Vec<SearchHit>,
    expected_user_id: i32,
    excluded_capture_id: Option<i32>,
) -> Vec<i32> {
    let mut seen_capture_ids = HashSet::new();
    let mut capture_ids = Vec::new();
    for hit in hits {
        let (user_id, capture_id) = match parse_fields(&hit.object_id) {
            Ok(ids) => ids,
            Err(err) => {
                tracing::warn!(id = hit.object_id, error = %err, "Failed parsing ids from object_id; dropping hit");
                continue;
            }
        };

        if user_id != expected_user_id {
            tracing::warn!(
                hit_user_id = user_id,
                expected_user_id,
                capture_id,
                "Dropping search hit from mismatched user"
            );
            continue;
        }
        if excluded_capture_id.is_some_and(|excluded| excluded == capture_id) {
            continue;
        }
        if seen_capture_ids.insert(capture_id) {
            capture_ids.push(capture_id);
        }
    }
    capture_ids
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
    use std::sync::{Arc, Mutex};

    use serde_json::{Map, Value};

    use super::*;
    use crate::search::{
        DataObject, Embedder, Embedding, QueryParams, SearchHit, SearchResultPage, Searcher, Unit,
        VectorStore, VectorUpsertResult,
    };

    #[test]
    fn parse_doc_id_extracts_user_capture() {
        assert_eq!(parse_fields("u1-c123").ok(), Some((1, 123)));
        assert!(parse_fields("u1-cabc").is_err());
        assert!(parse_fields("ufoo-c1").is_err());
        assert!(parse_fields("bad").is_err());
    }

    #[tokio::test]
    async fn search_returns_owned_deduped_capture_ids_and_clamps_limit() {
        let embedder = RecordingEmbedder::default();
        let searcher = RecordingSearcher::new(vec![
            hit("u1-c7", 0.9),
            hit("u1-c7", 0.8),
            hit("u1-c8", 0.7),
            hit("u2-c9", 0.99),
            hit("bad-id", 0.95),
        ]);
        let service = service(&embedder, &searcher, &RecordingVectorStore::default());
        let context = test_context(1);

        let ids = service
            .search(&context, "  ada  ", Some(5000))
            .await
            .unwrap();

        assert_eq!(ids, vec![7, 8]);
        assert_eq!(*embedder.queries.lock().unwrap(), vec!["ada".to_string()]);

        let calls = searcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "ada");
        assert_eq!(calls[0].1, 1000);
        assert_eq!(calls[0].2.get("user_id"), Some(&user_eq("1")));
    }

    #[tokio::test]
    async fn search_similar_excludes_its_own_capture_and_dedupes() {
        let searcher = RecordingSearcher::new(vec![
            hit("u1-c7", 0.9),
            hit("u1-c8", 0.8),
            hit("u1-c8", 0.7),
            hit("u2-c9", 0.99),
        ]);
        let store = RecordingVectorStore::with_embedding(Some(test_embedding()));
        let service = service(&RecordingEmbedder::default(), &searcher, &store);
        let context = test_context(1);

        let ids = service.search_similar(&context, 7, None).await.unwrap();

        assert_eq!(ids, vec![8]);
        assert_eq!(*store.fetches.lock().unwrap(), vec!["u1-c7".to_string()]);
        assert_eq!(searcher.calls.lock().unwrap()[0].1, 100);
    }

    #[tokio::test]
    async fn search_similar_without_an_embedding_returns_empty() {
        let searcher = RecordingSearcher::new(vec![hit("u1-c8", 0.8)]);
        let store = RecordingVectorStore::default();
        let service = service(&RecordingEmbedder::default(), &searcher, &store);
        let context = test_context(1);

        let ids = service.search_similar(&context, 7, None).await.unwrap();

        assert!(ids.is_empty());
        assert!(searcher.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_search_does_not_call_backends() {
        let embedder = RecordingEmbedder::default();
        let searcher = RecordingSearcher::new(vec![hit("u1-c1", 1.0)]);
        let service = service(&embedder, &searcher, &RecordingVectorStore::default());

        assert!(
            service
                .search(&test_context(1), "   ", None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(embedder.queries.lock().unwrap().is_empty());
        assert!(searcher.calls.lock().unwrap().is_empty());
    }

    fn service(
        embedder: &RecordingEmbedder,
        searcher: &RecordingSearcher,
        store: &RecordingVectorStore,
    ) -> SearchService {
        SearchService::from_parts(
            Box::new(embedder.clone()),
            Box::new(searcher.clone()),
            Box::new(store.clone()),
        )
    }

    fn test_context(user_id: i32) -> auth::Context {
        auth::Context::from(auth::DreamscrollAuthUser::new_test_session(user_id))
    }

    fn hit(object_id: &str, score: f64) -> SearchHit {
        SearchHit {
            object_id: object_id.to_string(),
            score,
        }
    }

    fn test_embedding() -> Embedding<f32, Unit> {
        Embedding::from_vec_normalizing(vec![1.0, 0.0]).unwrap()
    }

    fn user_eq(user_id: &str) -> Value {
        serde_json::json!({ "$eq": user_id })
    }

    type SearchCall = (String, u32, Map<String, Value>);

    #[derive(Clone, Default)]
    struct RecordingEmbedder {
        queries: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Embedder<Value, Embedding<f32, Unit>> for RecordingEmbedder {
        async fn embed_query(&self, query: &str) -> anyhow::Result<Embedding<f32, Unit>> {
            self.queries.lock().unwrap().push(query.to_string());
            Ok(test_embedding())
        }

        async fn embed_object(
            &self,
            _object: Value,
        ) -> anyhow::Result<crate::llms::InferenceOutcome<Embedding<f32, Unit>>> {
            anyhow::bail!("object embedding is not used")
        }
    }

    #[derive(Clone)]
    struct RecordingSearcher {
        hits: Vec<SearchHit>,
        calls: Arc<Mutex<Vec<SearchCall>>>,
    }

    impl RecordingSearcher {
        fn new(hits: Vec<SearchHit>) -> Self {
            Self {
                hits,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn record(&self, query: &str, params: &QueryParams) {
            self.calls.lock().unwrap().push((
                query.to_string(),
                params.limit,
                params.base_filter.clone(),
            ));
        }
    }

    #[async_trait::async_trait]
    impl Searcher<Embedding<f32, Unit>> for RecordingSearcher {
        async fn search_text(
            &self,
            _text_query: &str,
            _params: &QueryParams,
        ) -> anyhow::Result<SearchResultPage> {
            anyhow::bail!("text search is not used")
        }

        async fn search_embedding(
            &self,
            _embedding_query: &Embedding<f32, Unit>,
            params: &QueryParams,
        ) -> anyhow::Result<SearchResultPage> {
            self.record("embedding", params);
            Ok(page(self.hits.clone()))
        }

        async fn search_hybrid(
            &self,
            text_query: &str,
            _embedding_query: &Embedding<f32, Unit>,
            params: &QueryParams,
        ) -> anyhow::Result<SearchResultPage> {
            self.record(text_query, params);
            Ok(page(self.hits.clone()))
        }
    }

    #[derive(Clone, Default)]
    struct RecordingVectorStore {
        embedding: Option<Embedding<f32, Unit>>,
        fetches: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingVectorStore {
        fn with_embedding(embedding: Option<Embedding<f32, Unit>>) -> Self {
            Self {
                embedding,
                fetches: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl VectorStore<Embedding<f32, Unit>> for RecordingVectorStore {
        async fn upsert_object_embedding(
            &self,
            _object: &dyn DataObject,
            _embedding: &Embedding<f32, Unit>,
        ) -> anyhow::Result<VectorUpsertResult> {
            anyhow::bail!("upsert is not used")
        }

        async fn fetch_object_embedding(
            &self,
            object_id: &str,
        ) -> anyhow::Result<Option<Embedding<f32, Unit>>> {
            self.fetches.lock().unwrap().push(object_id.to_string());
            Ok(self.embedding.clone())
        }
    }

    fn page(hits: Vec<SearchHit>) -> SearchResultPage {
        SearchResultPage {
            hits,
            next_page_token: None,
        }
    }
}
