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
        let model_id = "grok-4-1-fast-reasoning".to_string();
        tracing::info!(model_id, "GrokFirestarter initialized");
        Self { api_key, model_id }
    }
}

#[async_trait::async_trait]
impl Firestarter for GrokFirestarter {
    fn name(&self) -> &str {
        "GrokFirestarter"
    }

    async fn spark(&self, captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<SparkResult> {
        let started = std::time::Instant::now();
        let input_capture_count = captures.len() as i32;
        let user_prompt = util::append_captures_to_user_prompt(prompt::PROMPT, captures);

        let request_body = ChatCompletionRequest {
            model: self.model_id.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
            // xAI search params docs: https://docs.x.ai/developers/rest-api-reference/inference/chat
            search_parameters: GrokSearchParameters {
                mode: "on".to_string(),
                return_citations: true,
                sources: vec!["web".to_string()],
            },
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

        let raw_response: serde_json::Value = response.json().await?;
        let parsed: ChatCompletionResponse = serde_json::from_value(raw_response.clone())?;
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
        let duration_ms = started.elapsed().as_millis() as i64;
        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_grok_usage(parsed.usage);
        let provider_grounding_json = parse_grok_grounding(&raw_response);

        Ok(SparkResult {
            spark: output,
            meta: SparkMeta {
                provider_name: self.name().to_string(),
                duration_ms,
                input_capture_count,
                input_tokens,
                output_tokens,
                total_tokens,
                provider_usage_json,
                provider_grounding_json,
            },
        })
    }
}

fn parse_grok_usage(
    usage: Option<serde_json::Value>,
) -> (Option<i32>, Option<i32>, Option<i32>, Option<String>) {
    let Some(usage) = usage else {
        return (None, None, None, None);
    };

    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let usage_json = serde_json::to_string(&usage).ok();
    (input_tokens, output_tokens, total_tokens, usage_json)
}

fn parse_grok_grounding(raw_response: &serde_json::Value) -> Option<String> {
    let first_choice = raw_response
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first());
    let message = first_choice.and_then(|choice| choice.get("message"));

    let annotations = message.and_then(|m| m.get("annotations")).cloned();
    let citations = message.and_then(|m| m.get("citations")).cloned();
    let sources = message.and_then(|m| m.get("sources")).cloned();

    let num_sources_used = raw_response
        .get("usage")
        .and_then(|usage| usage.get("num_sources_used"))
        .cloned();

    let web_search_calls = raw_response
        .get("usage")
        .and_then(|usage| usage.get("server_side_tool_usage_details"))
        .and_then(|details| details.get(1))
        .and_then(|tool_usage| tool_usage.get("web_search_calls"))
        .cloned();

    let has_grounding = annotations.is_some()
        || citations.is_some()
        || sources.is_some()
        || num_sources_used.is_some()
        || web_search_calls.is_some();

    if !has_grounding {
        return None;
    }

    let grounding = serde_json::json!({
        "annotations": annotations,
        "citations": citations,
        "sources": sources,
        "num_sources_used": num_sources_used,
        "web_search_calls": web_search_calls,
    });

    serde_json::to_string(&grounding).ok()
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
    search_parameters: GrokSearchParameters,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct GrokSearchParameters {
    mode: String,
    return_citations: bool,
    sources: Vec<String>,
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
    usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}
