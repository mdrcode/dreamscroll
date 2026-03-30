use serde::{Deserialize, Serialize};

use super::*;

#[async_trait::async_trait]
pub trait Searcher: Send + Sync {
    async fn search_query_embedding(
        &self,
        query_embedding: &QueryEmbedding,
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
