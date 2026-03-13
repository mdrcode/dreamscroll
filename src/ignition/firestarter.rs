use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Firestarter {
    fn name(&self) -> &str;
    async fn spark(&self, captures: Vec<api::CaptureInfo>) -> anyhow::Result<SparkResponse>;
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
