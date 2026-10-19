use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkMetaInfo {
    pub provider_name: String,
    pub backend_name: String,
    pub model_id: String,
    pub duration_ms: i64,
    pub tokens_in: Option<i64>,
    pub tokens_in_cached: Option<i64>,
    pub tokens_out: Option<i64>,
    pub params: serde_json::Value,
}
