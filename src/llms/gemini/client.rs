use anyhow::{Context, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use google_cloud_auth::credentials::{AccessTokenCredentials, Builder};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config;

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

/// User-input content accepted by an inference request.
pub enum GeminiInputPart<'a> {
    Text(&'a str),
    InlineImage { bytes: &'a [u8], mime_type: &'a str },
}

/// Per-call options. Prompts, schemas, tools, and retention are selected by the caller.
pub struct GeminiInteractionRequest<'a> {
    pub model_id: &'a str,
    pub input: &'a [GeminiInputPart<'a>],
    pub response_schema: Option<&'a Value>,
    pub tools: &'a [Value],
    pub store: bool,
}

/// Raw interaction steps remain available for future tool/agent flows.
#[derive(Debug, Deserialize)]
pub struct GeminiInteractionResponse {
    pub id: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub steps: Vec<Value>,
    pub usage: Option<Value>,
}

impl GeminiInferenceClient {
    pub fn from_config(cfg: &config::Config) -> anyhow::Result<Self> {
        let backend = match cfg
            .gemini_backend
            .context("GEMINI_BACKEND required when ILLUMINATOR=gemini")?
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

    pub async fn interact(
        &self,
        request: GeminiInteractionRequest<'_>,
    ) -> anyhow::Result<GeminiInteractionResponse> {
        if request.input.is_empty() {
            bail!("Gemini Interactions request must include input");
        }

        let content = request
            .input
            .iter()
            .map(|part| match part {
                GeminiInputPart::Text(text) => json!({
                    "type": "text",
                    "text": text,
                }),
                GeminiInputPart::InlineImage { bytes, mime_type } => json!({
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
        let mut body = json!({
            "model": request.model_id,
            "input": input,
            "store": request.store,
        });

        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools);
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

impl GeminiInteractionResponse {
    /// Returns text from the final model-output step, after any tool steps.
    pub fn output_text(&self) -> anyhow::Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn output_text_errors_when_interaction_requires_action() {
        let response: GeminiInteractionResponse = serde_json::from_value(json!({
            "status": "requires_action",
            "steps": [{"type": "function_call", "name": "lookup", "arguments": {}}]
        }))
        .unwrap();

        assert!(response.output_text().is_err());
    }
}
