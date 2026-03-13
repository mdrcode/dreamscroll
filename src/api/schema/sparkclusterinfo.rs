use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkClusterInfo {
    pub id: i32,
    pub title: String,
    pub summary: String,
    pub referenced_capture_ids: Vec<i32>,
    pub spark_links: Vec<super::SparkLinkInfo>,
}
