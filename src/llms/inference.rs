use serde_json::Value;

/// Input modalities accepted by application-level inference requests.
pub enum InferenceInput<'a> {
    Text(&'a str),
    Image { bytes: &'a [u8], mime_type: &'a str },
}

/// Provider-independent capabilities requested for an inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InferenceCapability {
    WebSearch,
}

/// Structured inference request used by application logic.
pub struct InferenceRequest<'a> {
    pub model_id: &'a str,
    pub input: &'a [InferenceInput<'a>],
    pub response_schema: Option<&'a Value>,
    pub capabilities: &'a [InferenceCapability],
}

/// Structured output and provider metadata returned by an inference client.
pub struct InferenceResponse {
    pub output: Value,
    pub provider_request_id: Option<String>,
    pub provider_usage_json: Option<Value>,
}

/// Provider-neutral boundary for model inference.
#[async_trait::async_trait]
pub trait InferenceClient: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn backend_name(&self) -> &'static str;

    async fn infer(&self, request: InferenceRequest<'_>) -> anyhow::Result<InferenceResponse>;
}
