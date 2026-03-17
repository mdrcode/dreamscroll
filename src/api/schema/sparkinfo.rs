use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{SparkClusterInfo, SparkMetaInfo};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkInfo {
    pub id: i32,
    pub created_at: DateTime<Utc>,
    pub created_at_human: String,
    pub input_capture_ids: Vec<i32>,
    pub meta: Option<SparkMetaInfo>,
    pub spark_clusters: Vec<SparkClusterInfo>,
}
