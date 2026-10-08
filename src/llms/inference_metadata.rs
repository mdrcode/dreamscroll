use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct InferenceMetadata {
    /// Correlates all inference outputs from one task attempt.
    pub inference_run_id: String,
    pub prompt_version: String,
    pub provider_name: String,
    pub backend_name: String,
    pub model_id: String,
    pub duration_ms: i64,
    /// Request identifier returned by the inference provider, when available.
    pub provider_request_id: Option<String>,
    pub provider_usage_json: Option<Value>,
}
