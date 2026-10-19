use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct InferenceMetadata {
    pub tokens_in: Option<i64>,
    pub tokens_in_cached: Option<i64>,
    pub tokens_out: Option<i64>,
    pub provider_name: String,
    pub backend_name: String,
    pub model_id: String,
    pub duration_ms: i64,
    /// Additional request configuration, including prompt_version; never response data.
    pub params: serde_json::Map<String, Value>,
    pub provider_metadata: serde_json::Map<String, Value>,
}
