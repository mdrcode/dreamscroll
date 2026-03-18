use serde::{Deserialize, Serialize};

use super::CapturePreviewInfo;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkClusterInfo {
    pub id: i32,
    pub title: String,
    pub summary: String,
    pub referenced_capture_ids: Vec<i32>,
    pub capture_previews: Vec<CapturePreviewInfo>,
    pub spark_links: Vec<super::SparkLinkInfo>,
}
