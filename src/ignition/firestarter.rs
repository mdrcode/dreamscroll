use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Firestarter: Send + Sync {
    fn name(&self) -> &str;
    async fn spark(&self, captures: Vec<api::CaptureInfo>) -> anyhow::Result<SparkResult>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkResult {
    pub spark: SparkResponse,
    pub meta: SparkMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkMeta {
    pub provider_name: String,
    pub duration_ms: i64,
    pub input_capture_count: i32,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub provider_usage_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkResponse {
    pub clusters: Vec<SparkRecommendedCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkRecommendedCluster {
    pub title: String,
    pub summary: String,
    pub capture_ids: Vec<i32>,
    pub recommended_links: Vec<SparkRecommendedLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SparkRecommendedLink {
    pub url: String,
    pub commentary: String,
}
