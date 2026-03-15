use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkMetaInfo {
    pub provider_name: String,
    pub duration_ms: i64,
    pub input_capture_count: i32,
    pub input_tokens: Option<i32>,
    pub output_tokens: Option<i32>,
    pub total_tokens: Option<i32>,
    pub provider_usage_json: Option<String>,
    pub provider_grounding_json: Option<String>,
}
