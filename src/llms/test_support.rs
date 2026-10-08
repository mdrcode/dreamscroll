use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use super::{InferenceClient, InferenceRequest, InferenceResponse};

pub(crate) struct TestInferenceClient {
    outputs: Vec<Value>,
    calls: AtomicUsize,
}

impl TestInferenceClient {
    pub(crate) fn new(outputs: Vec<Value>) -> Self {
        Self {
            outputs,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl InferenceClient for TestInferenceClient {
    fn provider_name(&self) -> &'static str {
        "test"
    }

    fn backend_name(&self) -> &'static str {
        "test"
    }

    async fn infer(&self, _request: InferenceRequest<'_>) -> anyhow::Result<InferenceResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let output = self
            .outputs
            .get(call)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("test inference outputs exhausted"))?;

        Ok(InferenceResponse {
            output,
            provider_request_id: Some(format!("test-inference-{call}")),
            provider_usage_json: None,
        })
    }
}
