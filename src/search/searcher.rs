use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait Searcher<E>: Send + Sync {
    async fn search_text(
        &self,
        text_query: &str,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;

    async fn search_embedding(
        &self,
        embedding_query: &E,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;

    async fn search_hybrid(
        &self,
        text_query: &str,
        embedding_query: &E,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    pub base_filter: serde_json::Map<String, serde_json::Value>,
    pub limit: u32,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub object_id: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultPage {
    pub hits: Vec<SearchHit>,
    pub next_page_token: Option<String>,
}
