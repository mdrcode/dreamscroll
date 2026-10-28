use std::time::Instant;

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{config, llms::*};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const DEVELOPER_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";

#[derive(Clone)]
enum Backend {
    DeveloperApi {
        api_key: String,
    },
    Vertex {
        endpoint: String,
        credentials: AccessTokenCredentials,
    },
}

/// Reusable REST client for Gemini model and agent interactions.
#[derive(Clone)]
pub struct GeminiInferenceClient {
    backend: Backend,
    http: Client,
}

/// Provider-specific response shape returned by the Gemini Interactions API.
#[derive(Debug, Deserialize)]
struct GeminiInteractionResponse {
    status: Option<String>,
    #[serde(default)]
    steps: Vec<Value>,
}

/// Capture the provider request id. Vertex often omits the body `id` when
/// `store` is false, so the response header is the only trace id.
fn capture_response_headers(
    headers: &reqwest::header::HeaderMap,
    metadata: &mut serde_json::Map<String, Value>,
) {
    if let Some(value) = headers.get("x-request-id").and_then(|v| v.to_str().ok()) {
        metadata.insert("provider_request_id".to_string(), json!(value));
    }
}

impl GeminiInferenceClient {
    pub fn from_config(cfg: &config::Config) -> anyhow::Result<Self> {
        let backend = match cfg
            .gemini_backend
            .context("GEMINI_BACKEND required for Gemini inference")?
        {
            config::GeminiBackend::DeveloperApi => Backend::DeveloperApi {
                api_key: cfg
                    .gemini_api_key
                    .as_deref()
                    .context("GEMINI_API_KEY required for the Gemini Developer API")?
                    .to_string(),
            },
            config::GeminiBackend::Vertex => {
                let credentials = Builder::default()
                    .with_scopes([CLOUD_PLATFORM_SCOPE])
                    .build_access_token_credentials()?;
                Backend::Vertex {
                    endpoint: format!(
                        "https://aiplatform.googleapis.com/v1beta1/projects/{}/locations/global/interactions",
                        cfg.gcloud_project_id
                    ),
                    credentials,
                }
            }
        };

        Ok(Self {
            backend,
            http: Client::new(),
        })
    }

    pub(crate) fn provider_name(&self) -> &'static str {
        "gemini"
    }

    pub(crate) fn backend_name(&self) -> &'static str {
        match &self.backend {
            Backend::DeveloperApi { .. } => "developer_api",
            Backend::Vertex { .. } => "vertex",
        }
    }

    fn interaction_body(&self, request: &InferenceRequest<'_>) -> anyhow::Result<Value> {
        if request.input.is_empty() {
            bail!("Gemini Interactions request must include input");
        }

        let content = request
            .input
            .iter()
            .map(|part| match part {
                InferenceInput::Text(text) => json!({
                    "type": "text",
                    "text": text,
                }),
                InferenceInput::Image { bytes, mime_type } => json!({
                    "type": "image",
                    "data": STANDARD.encode(bytes),
                    "mime_type": mime_type,
                }),
            })
            .collect::<Vec<_>>();

        let input = match &self.backend {
            Backend::DeveloperApi { .. } => Value::Array(content),
            Backend::Vertex { .. } => json!([{
                "type": "user_input",
                "content": content,
            }]),
        };
        // Keep user media in Dreamscroll's result storage, not the provider's interaction store.
        let mut body = json!({
            "model": request.model_id,
            "input": input,
            "store": false,
        });
        let tools = request
            .capabilities
            .iter()
            .map(|capability| match capability {
                InferenceCapability::WebSearch => json!({"type": "google_search"}),
            })
            .collect::<Vec<_>>();
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        if let Some(thinking_level) = request.thinking_level {
            body["generation_config"] = json!({"thinking_level": thinking_level});
        }

        if let Some(schema) = request.response_schema {
            match &self.backend {
                Backend::DeveloperApi { .. } => {
                    body["response_format"] = json!({
                        "type": "text",
                        "mime_type": "application/json",
                        "schema": schema,
                    });
                }
                Backend::Vertex { .. } => {
                    body["response_format"] = schema.clone();
                    body["response_mime_type"] = json!("application/json");
                }
            }
        }

        Ok(body)
    }

    async fn interact(
        &self,
        request: InferenceRequest<'_>,
    ) -> anyhow::Result<InferenceOutcome<Value>> {
        let body = self.interaction_body(&request)?;
        let (endpoint, api_key) = match &self.backend {
            Backend::DeveloperApi { api_key } => (DEVELOPER_API_URL, Some(api_key.as_str())),
            Backend::Vertex { endpoint, .. } => (endpoint.as_str(), None),
        };
        let model_id = request.model_id;
        let mut request = self.http.post(endpoint).json(&body);
        if let Some(api_key) = api_key {
            request = request.header("x-goog-api-key", api_key);
        } else if let Backend::Vertex { credentials, .. } = &self.backend {
            let token = credentials.access_token().await?.token;
            request = request.bearer_auth(token);
        }
        let started = Instant::now();
        let response = request.send().await?;
        let response_headers = response.headers().clone();
        let status = response.status();
        let response_text = response.text().await?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let response_json: Value = serde_json::from_str(&response_text)
            .context("Failed to parse Gemini Interactions response")?;
        let mut outcome = inference_response(
            response_json,
            status,
            self.backend_name(),
            model_id,
            duration_ms,
            body,
        );
        capture_response_headers(&response_headers, &mut outcome.metadata.provider_metadata);
        Ok(outcome)
    }
}
#[async_trait::async_trait]
impl InferenceClient for GeminiInferenceClient {
    fn provider_name(&self) -> &'static str {
        GeminiInferenceClient::provider_name(self)
    }

    fn backend_name(&self) -> &'static str {
        GeminiInferenceClient::backend_name(self)
    }

    async fn infer(
        &self,
        request: InferenceRequest<'_>,
    ) -> anyhow::Result<InferenceOutcome<Value>> {
        GeminiInferenceClient::interact(self, request).await
    }
}

impl GeminiInteractionResponse {
    /// Returns text from the final model-output step, after any tool steps.
    fn output_text(&self) -> anyhow::Result<String> {
        let step = self
            .steps
            .iter()
            .rev()
            .find(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
            .context("Gemini Interactions response has no model-output step")?;
        let content = step
            .get("content")
            .and_then(Value::as_array)
            .context("Gemini model-output step has no content array")?;

        let mut text = String::new();
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    text.push_str(part_text);
                }
            }
        }

        if text.trim().is_empty() {
            bail!(
                "Gemini model-output step contains no text (interaction status: {:?})",
                self.status
            );
        }
        Ok(text)
    }
}

fn inference_response(
    mut response: Value,
    status: reqwest::StatusCode,
    backend_name: &str,
    model_id: &str,
    duration_ms: i64,
    request_body: Value,
) -> InferenceOutcome<Value> {
    let mut metadata = InferenceMetadata {
        provider_name: "gemini".to_string(),
        backend_name: backend_name.to_string(),
        model_id: model_id.to_string(),
        duration_ms,
        ..Default::default()
    };
    if let Value::Object(mut params) = request_body {
        params.remove("input");
        params.remove("model");
        params.remove("response_format");
        params.remove("response_mime_type");
        metadata.params = params;
    }
    if let Some(id) = response
        .get_mut("id")
        .map(Value::take)
        .filter(Value::is_string)
    {
        metadata
            .provider_metadata
            .insert("provider_request_id".to_string(), id);
    }
    for key in ["created", "updated"] {
        if let Some(value) = response.get(key).cloned().filter(|value| !value.is_null()) {
            metadata
                .provider_metadata
                .insert(format!("provider_{key}"), value);
        }
    }
    if let Some(usage) = response
        .get_mut("usage")
        .map(Value::take)
        .filter(|value| !value.is_null())
    {
        metadata.tokens_in = usage.get("total_input_tokens").and_then(Value::as_i64);
        metadata.tokens_in_cached = usage.get("total_cached_tokens").and_then(Value::as_i64);
        metadata.tokens_out = usage.get("total_output_tokens").and_then(Value::as_i64);
        metadata.tokens_thought = usage.get("total_thought_tokens").and_then(Value::as_i64);
        metadata
            .provider_metadata
            .insert("provider_usage".to_string(), usage);
    }

    let output = if status.is_success() {
        serde_json::from_value::<GeminiInteractionResponse>(response)
            .context("Failed to parse Gemini Interactions response")
            .and_then(|interaction| {
                serde_json::from_str(&interaction.output_text()?)
                    .context("Gemini model output is not valid structured JSON")
            })
    } else {
        Err(anyhow::anyhow!(
            "Gemini Interactions API returned {status}: {response}"
        ))
    };
    InferenceOutcome {
        output,
        metadata,
        http_status_code: Some(status.as_u16()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn response_headers_capture_request_id() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-request-id", "req_123".parse().unwrap());
        let mut metadata = serde_json::Map::new();

        capture_response_headers(&headers, &mut metadata);

        assert_eq!(metadata["provider_request_id"], "req_123");
    }

    #[test]
    fn provider_error_carries_http_status() {
        let outcome = inference_response(
            serde_json::json!({"error": "resource exhausted"}),
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "vertex",
            "test-model",
            1,
            serde_json::json!({}),
        );
        assert_eq!(outcome.http_status_code, Some(429));
        let error = outcome.output.expect_err("provider error");
        assert!(format!("{error:#}").contains("429"));
    }

    use super::*;
    use crate::{illumination::IlluminationVersion, logic::illuminate::IlluminationTask};

    #[test]
    fn generic_request_uses_selected_model_and_maps_web_search_and_thinking() {
        let client = GeminiInferenceClient {
            backend: Backend::DeveloperApi {
                api_key: "test-key".to_string(),
            },
            http: Client::new(),
        };
        let input = [InferenceInput::Text("hello")];
        let capabilities = [InferenceCapability::WebSearch];
        let schema = json!({"type": "object", "properties": {"summary": {"type": "string"}}});

        for model_id in ["model-a", "model-b"] {
            for (level, expected) in [
                (None, None),
                (Some(gemini::ThinkingLevel::Minimal), Some("minimal")),
                (Some(gemini::ThinkingLevel::Low), Some("low")),
                (Some(gemini::ThinkingLevel::Medium), Some("medium")),
                (Some(gemini::ThinkingLevel::High), Some("high")),
            ] {
                let mut task = IlluminationTask::new(123, 1, model_id, IlluminationVersion::V1);
                if let Some(level) = level {
                    task.thinking_level = Some(level);
                }
                let payload = serde_json::to_value(task).expect("task payload should serialize");
                if expected.is_none() {
                    assert!(payload.get("thinking_level").is_none());
                }
                let task: IlluminationTask =
                    serde_json::from_value(payload).expect("task payload should deserialize");
                let request = InferenceRequest {
                    model_id: &task.model_id,
                    input: &input,
                    response_schema: Some(&schema),
                    capabilities: &capabilities,
                    thinking_level: task.thinking_level,
                };

                let body = client.interaction_body(&request).unwrap();
                assert_eq!(body["model"], model_id);
                assert!(!body["store"].as_bool().unwrap());
                assert_eq!(body["tools"], json!([{"type": "google_search"}]));
                assert_eq!(body["response_format"]["schema"], schema);
                let mut expected_params = json!({
                    "store": false,
                    "tools": [{"type": "google_search"}],
                });
                if let Some(expected) = expected {
                    let config = json!({"thinking_level": expected});
                    assert_eq!(body["generation_config"], config);
                    expected_params["generation_config"] = config;
                } else {
                    assert!(body.get("generation_config").is_none());
                }
                let outcome = inference_response(
                    json!({"usage": {"total_input_tokens": 1}}),
                    reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                    "developer_api",
                    model_id,
                    1,
                    body,
                );
                assert!(outcome.output.is_err());
                assert_eq!(
                    outcome.metadata.params,
                    *expected_params.as_object().unwrap()
                );
                assert_eq!(
                    outcome.metadata.provider_metadata["provider_usage"],
                    json!({"total_input_tokens": 1})
                );
            }
        }
    }

    #[test]
    fn output_text_uses_final_model_step_after_tools() {
        let response: GeminiInteractionResponse = serde_json::from_value(json!({
            "status": "completed",
            "steps": [
                {"type": "google_search_call", "arguments": {"queries": ["Ada Lovelace"]}},
                {"type": "google_search_result", "result": []},
                {"type": "model_output", "content": [
                    {"type": "text", "text": "{\"summary\":"},
                    {"type": "text", "text": "\"Ada\"}"}
                ]}
            ]
        }))
        .unwrap();

        assert_eq!(response.output_text().unwrap(), "{\"summary\":\"Ada\"}");
    }

    #[test]
    fn generic_response_contains_structured_output_and_provider_metadata() {
        let usage = json!({
            "total_input_tokens": 12,
            "total_cached_tokens": 3,
            "total_output_tokens": 7,
            "total_thought_tokens": 19,
            "total_tokens": 90,
            "input_tokens_by_modality": [{"modality": "image", "tokens": 4}]
        });
        let response = inference_response(
            json!({
                "id": "interaction-123",
                "status": "completed",
                "created": "2026-10-28T04:24:06Z",
                "updated": "2026-10-28T04:24:08Z",
                "usage": usage,
                "steps": [{"type": "model_output", "content": [
                    {"type": "text", "text": "{\"summary\":\"Ada\"}"}
                ]}]
            }),
            reqwest::StatusCode::OK,
            "vertex",
            "model-a",
            42,
            json!({"model": "model-a", "input": [{"text": "private prompt"}], "store": false,
                "generation_config": {"thinking_level": "high"},
                "response_format": {"type": "object"}, "response_mime_type": "application/json"}),
        );

        assert_eq!(response.output.unwrap(), json!({"summary": "Ada"}));
        assert_eq!(response.metadata.provider_name, "gemini");
        assert_eq!(response.metadata.backend_name, "vertex");
        assert_eq!(response.metadata.model_id, "model-a");
        assert_eq!(response.metadata.duration_ms, 42);
        assert_eq!(response.metadata.tokens_in, Some(12));
        assert_eq!(response.metadata.tokens_in_cached, Some(3));
        assert_eq!(response.metadata.tokens_out, Some(7));
        assert_eq!(response.metadata.tokens_thought, Some(19));
        assert_eq!(
            response.metadata.provider_metadata["provider_request_id"],
            json!("interaction-123")
        );
        assert_eq!(
            response.metadata.provider_metadata["provider_created"],
            json!("2026-10-28T04:24:06Z")
        );
        assert_eq!(
            response.metadata.provider_metadata["provider_updated"],
            json!("2026-10-28T04:24:08Z")
        );
        assert_eq!(response.metadata.provider_metadata["provider_usage"], usage);
        assert_eq!(
            response.metadata.params,
            json!({"store": false, "generation_config": {"thinking_level": "high"}})
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn output_text_errors_when_interaction_requires_action() {
        let response: GeminiInteractionResponse = serde_json::from_value(json!({
            "status": "requires_action",
            "steps": [{"type": "function_call", "name": "lookup", "arguments": {}}]
        }))
        .unwrap();

        assert!(response.output_text().is_err());
    }

    #[test]
    fn missing_usage_counts_remain_unknown_and_are_not_derived_from_totals() {
        for usage in [
            Value::Null,
            json!({}),
            json!({"total_tokens": 99}),
            json!({"total_thought_tokens": null}),
            json!({"total_thought_tokens": "12"}),
            json!({"total_thought_tokens": 1.5}),
            json!({"total_thought_tokens": true}),
            json!({"total_thought_tokens": []}),
            json!({"total_thought_tokens": {}}),
            json!({"total_thought_tokens": u64::MAX}),
        ] {
            let response = inference_response(
                json!({"usage": usage}),
                reqwest::StatusCode::OK,
                "developer_api",
                "model-a",
                1,
                json!({}),
            );
            assert_eq!(response.metadata.tokens_in, None);
            assert_eq!(response.metadata.tokens_in_cached, None);
            assert_eq!(response.metadata.tokens_out, None);
            assert_eq!(response.metadata.tokens_thought, None);
            assert!(
                !response
                    .metadata
                    .provider_metadata
                    .contains_key("provider_request_id")
            );
            if usage.is_null() {
                assert!(
                    !response
                        .metadata
                        .provider_metadata
                        .contains_key("provider_usage")
                );
            } else {
                assert_eq!(response.metadata.provider_metadata["provider_usage"], usage);
            }
        }
        let response = inference_response(
            json!({}),
            reqwest::StatusCode::OK,
            "developer_api",
            "model-a",
            1,
            json!({}),
        );
        assert_eq!(response.metadata.tokens_in, None);
        assert_eq!(response.metadata.tokens_in_cached, None);
        assert_eq!(response.metadata.tokens_out, None);
        assert_eq!(response.metadata.tokens_thought, None);
        assert!(response.metadata.params.is_empty());
    }

    #[test]
    fn usage_counts_preserve_large_values_and_explicit_zero() {
        for tokens_thought in [0_i64, 5_000_000_000, i64::MAX] {
            let usage = json!({
                "total_input_tokens": 4_000_000_000_i64,
                "total_cached_tokens": 3_000_000_000_i64,
                "total_output_tokens": 0,
                "total_thought_tokens": tokens_thought
            });
            let response = inference_response(
                json!({"usage": usage}),
                reqwest::StatusCode::OK,
                "developer_api",
                "model-a",
                1,
                json!({}),
            );
            assert_eq!(response.metadata.tokens_in, Some(4_000_000_000));
            assert_eq!(response.metadata.tokens_in_cached, Some(3_000_000_000));
            assert_eq!(response.metadata.tokens_out, Some(0));
            assert_eq!(response.metadata.tokens_thought, Some(tokens_thought));
            assert_eq!(response.metadata.provider_metadata["provider_usage"], usage);
        }
    }

    #[test]
    fn output_interpretation_errors_preserve_provider_usage() {
        for steps in [
            json!([]),
            json!([{"type": "model_output", "content": [{"type": "text", "text": "not json"}]}]),
            Value::Null,
        ] {
            let response = inference_response(
                json!({
                    "id": "interaction-failed",
                    "usage": {"total_input_tokens": 12, "total_output_tokens": 7,
                        "total_thought_tokens": 23},
                    "steps": steps
                }),
                reqwest::StatusCode::OK,
                "developer_api",
                "model-a",
                1,
                json!({"model": "model-a", "input": [{"text": "private prompt"}],
                    "store": false, "tools": [{"type": "google_search"}]}),
            );
            assert!(response.output.is_err());
            assert_eq!(response.metadata.tokens_in, Some(12));
            assert_eq!(response.metadata.tokens_out, Some(7));
            assert_eq!(response.metadata.tokens_thought, Some(23));
            assert_eq!(
                response.metadata.provider_metadata["provider_request_id"],
                json!("interaction-failed")
            );
            assert_eq!(
                response.metadata.provider_metadata["provider_usage"],
                json!({"total_input_tokens": 12, "total_output_tokens": 7,
                    "total_thought_tokens": 23})
            );
            assert_eq!(
                response.metadata.params,
                json!({"store": false,
                "tools": [{"type": "google_search"}]})
                .as_object()
                .unwrap()
                .clone()
            );
        }
    }

    #[test]
    fn http_error_json_preserves_provider_usage() {
        let response = inference_response(
            json!({
                "id": "interaction-error",
                "error": {"message": "provider failed"},
                "usage": {"total_input_tokens": 12, "total_cached_tokens": 2,
                    "total_output_tokens": 7, "total_thought_tokens": 29}
            }),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "developer_api",
            "model-a",
            1,
            json!({"model": "model-a", "input": [{"text": "private prompt"}], "store": false}),
        );
        assert!(response.output.unwrap_err().to_string().contains("500"));
        assert_eq!(response.metadata.tokens_in, Some(12));
        assert_eq!(response.metadata.tokens_in_cached, Some(2));
        assert_eq!(response.metadata.tokens_out, Some(7));
        assert_eq!(response.metadata.tokens_thought, Some(29));
        assert_eq!(
            response.metadata.provider_metadata["provider_request_id"],
            json!("interaction-error")
        );
        assert_eq!(
            response.metadata.provider_metadata["provider_usage"],
            json!({"total_input_tokens": 12, "total_cached_tokens": 2,
                "total_output_tokens": 7, "total_thought_tokens": 29})
        );
        assert_eq!(
            response.metadata.params,
            json!({"store": false}).as_object().unwrap().clone()
        );
    }
}
