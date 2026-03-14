use serde::{Deserialize, Serialize};

use super::{SparkClusterInfo, SparkMetaInfo};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkInfo {
    pub id: i32,
    pub input_capture_ids: Vec<i32>,
    pub meta: Option<SparkMetaInfo>,
    pub spark_clusters: Vec<SparkClusterInfo>,
}
