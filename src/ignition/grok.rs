use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;

pub struct GrokFirestarter {
    api_key: String,
    model_id: String,
}

impl GrokFirestarter {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model_id: "grok-4-1-fast-reasoning".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Firestarter for GrokFirestarter {
    fn name(&self) -> &str {
        "GrokFirestarter"
    }

    async fn spark(&self, captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<SparkResponse> {
        let user_prompt = util::append_captures_to_user_prompt(prompt::PROMPT, captures);

        let request_body = ChatCompletionRequest {
            model: self.model_id.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
            response_format: ResponseFormat {
                response_type: "json_schema".to_string(),
                json_schema: JsonSchemaSpec {
                    name: "spark_output".to_string(),
                    strict: true,
                    schema: spark_output_schema_grok(),
                },
            },
        };

        let client = Client::new();
        let response = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("XAI API error: {}", error_text);
        }

        let parsed: ChatCompletionResponse = response.json().await?;
        let content = parsed
            .choices
            .first()
            .map(|c| c.message.content.trim())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            anyhow::bail!("XAI API returned an empty response body");
        }

        let output: SparkResponse = match serde_json::from_str(&content) {
            Ok(it) => it,
            Err(err) => {
                let dump_path =
                    util::dump_raw_response_to_tmp(&content, "dreamscroll-spark-response");
                return Err(match dump_path {
                    Ok(path) => anyhow::anyhow!(
                        "Failed to parse structured spark response: {}. Raw JSON saved to {}",
                        err,
                        path.display()
                    ),
                    Err(dump_err) => anyhow::anyhow!(
                        "Failed to parse structured spark response: {}. Also failed to write raw JSON to tmp: {}",
                        err,
                        dump_err
                    ),
                });
            }
        };
        Ok(output)
    }
}

fn spark_output_schema_grok() -> serde_json::Value {
    json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["clusters"],
            "properties": {
                    "clusters": {
                            "type": "array",
                            "items": {
                                    "type": "object",
                                    "additionalProperties": false,
                                        "required": ["title", "summary", "capture_ids", "recommended_links"],
                                    "properties": {
                                            "title": { "type": "string" },
                                            "summary": { "type": "string" },
                                            "capture_ids": {
                                                    "type": "array",
                                                    "items": { "type": "integer" }
                                            },
                                            "recommended_links": {
                                                    "type": "array",
                                                    "items": {
                                                            "type": "object",
                                                            "additionalProperties": false,
                                                    "required": ["url", "commentary"],
                                                            "properties": {
                                                                    "url": { "type": "string" },
                                                                    "commentary": { "type": "string" }
                                                            }
                                                    }
                                            }
                                    }
                            }
                    }
            }
    })
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    response_type: String,
    json_schema: JsonSchemaSpec,
}

#[derive(Serialize)]
struct JsonSchemaSpec {
    name: String,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}
