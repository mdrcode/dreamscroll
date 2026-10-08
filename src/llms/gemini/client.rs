use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    config,
    llms::{
        InferenceCapability, InferenceClient, InferenceInput, InferenceRequest, InferenceResponse,
    },
};

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
    id: Option<String>,
    status: Option<String>,
    #[serde(default)]
    steps: Vec<Value>,
    usage: Option<Value>,
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
    ) -> anyhow::Result<GeminiInteractionResponse> {
        let body = self.interaction_body(&request)?;
        let (endpoint, api_key) = match &self.backend {
            Backend::DeveloperApi { api_key } => (DEVELOPER_API_URL, Some(api_key.as_str())),
            Backend::Vertex { endpoint, .. } => (endpoint.as_str(), None),
        };
        let mut request = self.http.post(endpoint).json(&body);
        if let Some(api_key) = api_key {
            request = request.header("x-goog-api-key", api_key);
        } else if let Backend::Vertex { credentials, .. } = &self.backend {
            let token = credentials.access_token().await?.token;
            request = request.bearer_auth(token);
        }

        let response = request.send().await?;
        let status = response.status();
        let response_text = response.text().await?;
        if !status.is_success() {
            bail!("Gemini Interactions API returned {status}: {response_text}");
        }

        serde_json::from_str(&response_text).context("Failed to parse Gemini Interactions response")
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

    async fn infer(&self, request: InferenceRequest<'_>) -> anyhow::Result<InferenceResponse> {
        let interaction = GeminiInferenceClient::interact(self, request).await?;
        interaction.into_inference_response()
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

    fn into_inference_response(self) -> anyhow::Result<InferenceResponse> {
        let output = serde_json::from_str(&self.output_text()?)
            .context("Gemini model output is not valid structured JSON")?;
        Ok(InferenceResponse {
            output,
            provider_request_id: self.id,
            provider_usage_json: self.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{illumination::IlluminationVersion, logic::illuminate::IlluminationTask};

    #[test]
    fn generic_request_uses_selected_model_and_maps_web_search() {
        let client = GeminiInferenceClient {
            backend: Backend::DeveloperApi {
                api_key: "test-key".to_string(),
            },
            http: Client::new(),
        };
        let input = [InferenceInput::Text("hello")];
        let capabilities = [InferenceCapability::WebSearch];

        for model_id in ["model-a", "model-b"] {
            let payload = serde_json::to_vec(&IlluminationTask::new(
                123,
                model_id,
                IlluminationVersion::V1,
            ))
            .expect("task payload should serialize");
            let task: IlluminationTask =
                serde_json::from_slice(&payload).expect("task payload should deserialize");
            let request = InferenceRequest {
                model_id: &task.model_id,
                input: &input,
                response_schema: None,
                capabilities: &capabilities,
            };

            let body = client.interaction_body(&request).unwrap();
            assert_eq!(body["model"], model_id);
            assert!(!body["store"].as_bool().unwrap());
            assert_eq!(body["tools"], json!([{"type": "google_search"}]));
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
        let response: GeminiInteractionResponse = serde_json::from_value(json!({
            "id": "interaction-123",
            "status": "completed",
            "usage": {"total_tokens": 7},
            "steps": [{"type": "model_output", "content": [
                {"type": "text", "text": "{\"summary\":\"Ada\"}"}
            ]}]
        }))
        .unwrap();

        let response = response.into_inference_response().unwrap();
        assert_eq!(response.output, json!({"summary": "Ada"}));
        assert_eq!(
            response.provider_request_id.as_deref(),
            Some("interaction-123")
        );
        assert_eq!(
            response.provider_usage_json,
            Some(json!({"total_tokens": 7}))
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
}
