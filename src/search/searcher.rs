use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait Searcher<E>: Send + Sync {
    async fn search_text(
        &self,
        query_text: &str,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;

    async fn search_embedding(
        &self,
        query_embedding: &E,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;

    async fn search_hybrid(
        &self,
        query_text: &str,
        query_embedding: &E,
        params: &QueryParams,
    ) -> anyhow::Result<SearchResultPage>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    pub user_id: i32,
    pub limit: u32,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc_id: String, // vector store's native doc ID
    pub user_id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultPage {
    pub hits: Vec<SearchHit>,
    pub next_page_token: Option<String>,
}
