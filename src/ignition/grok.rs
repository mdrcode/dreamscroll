use reqwest::Client;
use serde::Serialize;
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

        let request_body = ResponsesRequest {
            model: self.model_id.clone(),
            input: vec![InputMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
            // xAI Agent Tools docs: https://docs.x.ai/developers/tools/overview
            tools: vec![GrokTool::WebSearch],
            text: ResponseText {
                format: ResponseTextFormat {
                    format_type: "json_schema".to_string(),
                    name: "spark_output".to_string(),
                    strict: true,
                    schema: spark_output_schema_grok(),
                },
            },
        };

        let client = Client::new();
        let response = client
            .post("https://api.x.ai/v1/responses")
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
        let raw_response_text = raw_response.to_string();
        let content = extract_grok_output_text(&raw_response)
            .map(str::trim)
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            anyhow::bail!(
                "XAI API returned an empty structured response. Response excerpt: {}",
                util::truncate_for_error(&raw_response_text, 600)
            );
        }

        let output: SparkResponse = match serde_json::from_str(&content) {
            Ok(it) => it,
            Err(err) => anyhow::bail!(
                "Failed to parse structured spark response: {}. Content excerpt: {}",
                err,
                util::truncate_for_error(&content, 600)
            ),
        };
        let duration_ms = started.elapsed().as_millis() as i64;
        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_grok_usage_from_response(&raw_response);
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

fn parse_grok_usage_from_response(
    raw_response: &serde_json::Value,
) -> (Option<i32>, Option<i32>, Option<i32>, Option<String>) {
    let usage = raw_response.get("usage").cloned();
    let Some(usage) = usage else {
        return (None, None, None, None);
    };

    let input_tokens = usage
        .get("input_tokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let output_tokens = usage
        .get("output_tokens")
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
    let annotations = find_grok_output_text_part(raw_response)
        .and_then(|part| part.get("annotations"))
        .cloned();
    let citations = raw_response.get("citations").cloned();
    let sources = raw_response.get("sources").cloned();

    let num_sources_used = raw_response
        .get("usage")
        .and_then(|usage| usage.get("num_sources_used"))
        .cloned();

    let web_search_calls = raw_response
        .get("usage")
        .and_then(|usage| usage.get("server_side_tool_usage_details"))
        .and_then(|details| details.get("web_search_calls"))
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

fn extract_grok_output_text(raw_response: &serde_json::Value) -> Option<&str> {
    if let Some(text) = raw_response.get("output_text").and_then(|v| v.as_str()) {
        if !text.trim().is_empty() {
            return Some(text);
        }
    }

    find_grok_output_text_part(raw_response)
        .and_then(|part| part.get("text"))
        .and_then(|v| v.as_str())
}

fn find_grok_output_text_part(raw_response: &serde_json::Value) -> Option<&serde_json::Value> {
    raw_response
        .get("output")
        .and_then(|v| v.as_array())
        .and_then(|output| {
            output.iter().find_map(|item| {
                let is_message = item.get("type").and_then(|v| v.as_str()) == Some("message");
                if !is_message {
                    return None;
                }

                item.get("content")
                    .and_then(|v| v.as_array())
                    .and_then(|content| {
                        content.iter().find(|part| {
                            part.get("type").and_then(|v| v.as_str()) == Some("output_text")
                        })
                    })
            })
        })
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
struct ResponsesRequest {
    model: String,
    input: Vec<InputMessage>,
    tools: Vec<GrokTool>,
    text: ResponseText,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum GrokTool {
    #[serde(rename = "web_search")]
    WebSearch,
}

#[derive(Serialize)]
struct ResponseText {
    format: ResponseTextFormat,
}

#[derive(Serialize)]
struct ResponseTextFormat {
    #[serde(rename = "type")]
    format_type: String,
    name: String,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct InputMessage {
    role: String,
    content: String,
}
