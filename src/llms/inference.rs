use serde_json::Value;

use super::{InferenceMetadata, gemini};

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
    pub thinking_level: Option<gemini::ThinkingLevel>,
}

#[derive(Debug)]
pub struct InferenceOutcome<T> {
    pub output: anyhow::Result<T>,
    pub metadata: InferenceMetadata,
    /// Provider HTTP status for this request, when the provider is HTTP-based.
    /// Retained on failure so callers can record the most specific status.
    pub http_status_code: Option<u16>,
}

/// Provider-neutral boundary for model inference.
#[async_trait::async_trait]
pub trait InferenceClient: Send + Sync {
    fn provider_name(&self) -> &'static str;
    fn backend_name(&self) -> &'static str;

    /// Outer errors lack usable metadata; output errors retain it in the outcome.
    async fn infer(&self, request: InferenceRequest<'_>)
    -> anyhow::Result<InferenceOutcome<Value>>;
}
