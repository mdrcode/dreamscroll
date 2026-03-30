use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait Searcher: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> anyhow::Result<SearchResultPage>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub user_id: i32,
    pub text: String,
    pub limit: u32,
    pub page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub corpus_doc_id: String,
    pub capture_id: i32,
    pub illumination_id: i32,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultPage {
    pub hits: Vec<SearchHit>,
    pub next_page_token: Option<String>,
}
