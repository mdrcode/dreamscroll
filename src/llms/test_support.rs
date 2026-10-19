use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use super::{InferenceClient, InferenceMetadata, InferenceOutcome, InferenceRequest};

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

    async fn infer(
        &self,
        request: InferenceRequest<'_>,
    ) -> anyhow::Result<InferenceOutcome<Value>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let output = self
            .outputs
            .get(call)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("test inference outputs exhausted"))?;

        Ok(InferenceOutcome {
            output: Ok(output),
            metadata: InferenceMetadata {
                provider_name: self.provider_name().to_string(),
                backend_name: self.backend_name().to_string(),
                model_id: request.model_id.to_string(),
                params: serde_json::Map::from_iter([(
                    "provider_request_id".to_string(),
                    Value::String(format!("test-inference-{call}")),
                )]),
                ..Default::default()
            },
        })
    }
}
