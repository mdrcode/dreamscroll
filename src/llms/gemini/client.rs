use std::time::Instant;

use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde_json::{Value, json};

use crate::{config, llms::*};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const DEVELOPER_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";

/// Ceiling on generated tokens per request, thinking included.
///
/// The provider's default is 65,536, and a degenerate generation can spend all of it:
/// capture 766 emitted 65,090 tokens over 335 s at 30x a normal run's cost, then the
/// JSON truncated mid-string. This bounds that to an eighth.
///
/// It is not a cost lever — over 1,222 runs, 4,096 would save $0.14 of $7.39 — so the
/// number is chosen to avoid killing good runs. Thinking counts toward the limit, and
/// the median run bills 1,440 tokens while only 2 of 1,222 exceed 8,192; those two are
/// also the worse answers (capture 743's 11,185-thought run invented `r/musictheory`
/// entities, where its 1,461-thought run named Chopin's Nocturne correctly).
const MAX_OUTPUT_TOKENS: u32 = 8_192;

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

/// Provider-specific response shape, borrowed so a parse failure can retain the body.
#[derive(Debug)]
struct GeminiInteractionResponse<'a> {
    status: Option<&'a str>,
    steps: &'a [Value],
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
        // Media normally stays in Dreamscroll's storage; `store` is a debug opt-in.
        let mut body = json!({
            "model": request.model_id,
            "input": input,
            "store": request.gemini.store,
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

        let mut generation_config = json!({ "max_output_tokens": MAX_OUTPUT_TOKENS });
        if let Some(thinking_level) = request.gemini.thinking_level {
            generation_config["thinking_level"] = json!(thinking_level);
        }
        if request.gemini.thinking_summaries {
            generation_config["thinking_summaries"] = json!("auto");
        }
        body["generation_config"] = generation_config;

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

fn parse_interaction(response: &Value) -> Result<GeminiInteractionResponse<'_>, serde_json::Error> {
    let status = match response.get("status") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => return Err(serde::de::Error::custom("Gemini status must be a string")),
    };
    let steps = match response.get("steps") {
        None => &[],
        Some(Value::Array(steps)) => steps.as_slice(),
        Some(_) => return Err(serde::de::Error::custom("Gemini steps must be an array")),
    };
    Ok(GeminiInteractionResponse { status, steps })
}

impl GeminiInteractionResponse<'_> {
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
            if part.get("type").and_then(Value::as_str) == Some("text")
                && let Some(part_text) = part.get("text").and_then(Value::as_str)
            {
                text.push_str(part_text);
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
    // Grounding is billed per query, and the steps are the only place that count lives: the
    // model-output text cannot tell us whether a run searched.
    let queries: usize = response
        .get("steps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter(|step| step["type"] == "google_search_call")
                .map(|step| step["arguments"]["queries"].as_array().map_or(0, Vec::len))
                .sum()
        })
        .unwrap_or(0);
    if queries > 0 {
        metadata
            .provider_metadata
            .insert("provider_search_queries".to_string(), json!(queries));
    }

    let output = if status.is_success() {
        match parse_interaction(&response) {
            Ok(interaction) => match interaction.output_text() {
                Ok(text) => match serde_json::from_str(&text) {
                    Ok(value) => Ok(value),
                    Err(error) => {
                        metadata.raw_output = Some(text);
                        Err(error).context("Gemini model output is not valid structured JSON")
                    }
                },
                Err(error) => {
                    metadata.raw_output = Some(response.to_string());
                    Err(error)
                }
            },
            Err(error) => {
                metadata.raw_output = Some(response.to_string());
                Err(error).context("Failed to parse Gemini Interactions response")
            }
        }
    } else {
        metadata.raw_output = Some(response.to_string());
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
    use super::*;
    use crate::llms::gemini::GeminiInferenceOptions;

    fn developer_api_client() -> GeminiInferenceClient {
        GeminiInferenceClient {
            backend: Backend::DeveloperApi {
                api_key: "test-key".to_string(),
            },
            http: Client::new(),
        }
    }

    #[test]
    fn response_headers_capture_request_id() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-request-id", "req_123".parse().unwrap());
        let mut metadata = serde_json::Map::new();

        capture_response_headers(&headers, &mut metadata);

        assert_eq!(metadata["provider_request_id"], "req_123");
    }

    #[test]
    fn request_body_maps_model_tools_and_thinking_level() {
        let client = developer_api_client();
        let input = [InferenceInput::Text("hello")];
        let capabilities = [InferenceCapability::WebSearch];
        let schema = json!({"type": "object", "properties": {"summary": {"type": "string"}}});

        for (level, expected) in [
            (None, None),
            (Some(gemini::ThinkingLevel::Low), Some("low")),
            (Some(gemini::ThinkingLevel::Medium), Some("medium")),
            (Some(gemini::ThinkingLevel::High), Some("high")),
        ] {
            let gemini = GeminiInferenceOptions {
                thinking_level: level,
                ..Default::default()
            };
            let request = InferenceRequest {
                model_id: "model-a",
                input: &input,
                response_schema: Some(&schema),
                capabilities: &capabilities,
                gemini: &gemini,
            };

            let body = client.interaction_body(&request).unwrap();
            assert_eq!(body["model"], "model-a");
            // User media belongs in Dreamscroll's storage, not the provider's.
            assert_eq!(body["store"], json!(false));
            assert_eq!(body["tools"], json!([{"type": "google_search"}]));
            assert_eq!(body["response_format"]["schema"], schema);
            match expected {
                Some(level) => {
                    assert_eq!(body["generation_config"]["thinking_level"], json!(level))
                }
                None => assert!(body["generation_config"].get("thinking_level").is_none()),
            }
            // Every request carries the ceiling: a degenerate generation otherwise runs
            // unbounded (65k tokens, 335 s, truncated JSON on capture 766).
            assert_eq!(body["generation_config"]["max_output_tokens"], json!(8192));
        }
    }

    #[test]
    fn request_body_omits_tools_without_web_search_capability() {
        let client = developer_api_client();
        let input = [InferenceInput::Text("hello")];
        let capabilities: [InferenceCapability; 0] = [];
        let gemini = GeminiInferenceOptions::default();
        let request = InferenceRequest {
            model_id: "model-a",
            input: &input,
            response_schema: None,
            capabilities: &capabilities,
            gemini: &gemini,
        };

        let body = client.interaction_body(&request).unwrap();
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn request_body_honors_the_store_flag() {
        let client = developer_api_client();
        let input = [InferenceInput::Text("hello")];
        let capabilities: [InferenceCapability; 0] = [];
        let gemini = GeminiInferenceOptions {
            store: true,
            ..Default::default()
        };
        let request = InferenceRequest {
            model_id: "model-a",
            input: &input,
            response_schema: None,
            capabilities: &capabilities,
            gemini: &gemini,
        };

        let body = client.interaction_body(&request).unwrap();
        assert_eq!(body["store"], json!(true));
    }

    #[test]
    fn request_body_requests_thinking_summaries() {
        let client = developer_api_client();
        let input = [InferenceInput::Text("hello")];
        let capabilities: [InferenceCapability; 0] = [];
        let gemini = GeminiInferenceOptions {
            store: true,
            thinking_summaries: true,
            ..Default::default()
        };
        let request = InferenceRequest {
            model_id: "model-a",
            input: &input,
            response_schema: None,
            capabilities: &capabilities,
            gemini: &gemini,
        };

        let body = client.interaction_body(&request).unwrap();
        assert_eq!(
            body["generation_config"]["thinking_summaries"],
            json!("auto")
        );
    }

    #[test]
    fn request_body_rejects_missing_input() {
        let client = developer_api_client();
        let input: [InferenceInput; 0] = [];
        let capabilities: [InferenceCapability; 0] = [];
        let gemini = GeminiInferenceOptions::default();
        let request = InferenceRequest {
            model_id: "model-a",
            input: &input,
            response_schema: None,
            capabilities: &capabilities,
            gemini: &gemini,
        };

        assert!(client.interaction_body(&request).is_err());
    }

    #[test]
    fn response_counts_grounding_queries_across_calls() {
        let searching = json!({
            "status": "completed",
            "steps": [
                {"type": "thought", "signature": "sig"},
                {"type": "google_search_call", "arguments": {"queries": ["Ada Lovelace", "Analytical Engine"]}},
                {"type": "google_search_result", "result": []},
                {"type": "google_search_call", "arguments": {"queries": ["Charles Babbage"]}},
                {"type": "model_output", "content": [{"type": "text", "text": "{}"}]}
            ]
        });
        let quiet = json!({
            "status": "completed",
            "steps": [{"type": "model_output", "content": [{"type": "text", "text": "{}"}]}]
        });

        for (response, expected) in [(searching, Some(3)), (quiet, None)] {
            let outcome = inference_response(
                response,
                reqwest::StatusCode::OK,
                "vertex",
                "gemini-3.8-flash",
                10,
                json!({}),
            );
            let recorded = outcome
                .metadata
                .provider_metadata
                .get("provider_search_queries")
                .cloned();
            assert_eq!(recorded, expected.map(|n| json!(n)));
        }
    }

    #[test]
    fn output_text_uses_final_model_step_after_tools() {
        let value = json!({
            "status": "completed",
            "steps": [
                {"type": "google_search_call", "arguments": {"queries": ["Ada Lovelace"]}},
                {"type": "google_search_result", "result": []},
                {"type": "model_output", "content": [
                    {"type": "text", "text": "{\"summary\":"},
                    {"type": "text", "text": "\"Ada\"}"}
                ]}
            ]
        });
        let response = parse_interaction(&value).unwrap();

        assert_eq!(response.output_text().unwrap(), "{\"summary\":\"Ada\"}");
    }

    #[test]
    fn response_maps_output_tokens_and_provider_metadata() {
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
        assert!(response.metadata.raw_output.is_none());
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
        // Recorded params must not leak the prompt or the provider request fields.
        assert_eq!(
            response.metadata.params,
            json!({"store": false, "generation_config": {"thinking_level": "high"}})
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn invalid_model_json_is_retained_as_raw_output() {
        let raw = "{\"summary\":";
        let outcome = inference_response(
            json!({
                "status": "completed",
                "steps": [{"type": "model_output", "content": [
                    {"type": "text", "text": raw}
                ]}]
            }),
            reqwest::StatusCode::OK,
            "vertex",
            "model-a",
            1,
            json!({}),
        );

        assert!(outcome.output.is_err());
        assert_eq!(outcome.metadata.raw_output.as_deref(), Some(raw));
    }

    #[test]
    fn provider_error_body_is_retained_as_raw_output() {
        let body = json!({"error": {"code": 500, "message": "boom"}});
        let outcome = inference_response(
            body.clone(),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "vertex",
            "model-a",
            1,
            json!({}),
        );

        assert!(outcome.output.is_err());
        assert_eq!(outcome.metadata.raw_output, Some(body.to_string()));
    }

    #[test]
    fn a_response_without_model_output_retains_the_body() {
        let body = json!({"status": "requires_action", "steps": []});
        let outcome = inference_response(
            body.clone(),
            reqwest::StatusCode::OK,
            "vertex",
            "model-a",
            1,
            json!({}),
        );

        assert!(outcome.output.is_err());
        assert_eq!(outcome.metadata.raw_output, Some(body.to_string()));
    }

    #[test]
    fn an_invalid_response_envelope_retains_the_body() {
        let body = json!({"status": "completed", "steps": "not-an-array"});
        let outcome = inference_response(
            body.clone(),
            reqwest::StatusCode::OK,
            "vertex",
            "model-a",
            1,
            json!({}),
        );

        assert!(outcome.output.is_err());
        assert_eq!(outcome.metadata.raw_output, Some(body.to_string()));
    }

    #[test]
    fn output_text_errors_when_interaction_requires_action() {
        let value = json!({
            "status": "requires_action",
            "steps": [{"type": "function_call", "name": "lookup", "arguments": {}}]
        });
        let response = parse_interaction(&value).unwrap();

        assert!(response.output_text().is_err());
    }

    #[test]
    fn missing_usage_counts_remain_unknown_and_are_not_derived_from_totals() {
        for usage in [
            Value::Null,
            json!({}),
            json!({"total_tokens": 99}),
            json!({"total_thought_tokens": "12"}),
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
    }

    #[test]
    fn usage_counts_preserve_large_values_and_explicit_zero() {
        for tokens_thought in [0_i64, i64::MAX] {
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
        assert_eq!(response.http_status_code, Some(500));
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
