use serde::{Deserialize, Serialize};

use super::SparkClusterInfo;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkInfo {
    pub id: i32,
    pub spark_clusters: Vec<SparkClusterInfo>,
}
