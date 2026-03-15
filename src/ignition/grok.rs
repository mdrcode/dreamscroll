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
        let raw_response_text = serde_json::to_string_pretty(&raw_response)
            .unwrap_or_else(|_| raw_response.to_string());
        let content = extract_grok_output_text(&raw_response)
            .map(str::trim)
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            let dump_path = util::dump_raw_response_to_tmp(
                &raw_response_text,
                "dreamscroll-grok-empty-response",
            );

            return Err(match dump_path {
                Ok(path) => anyhow::anyhow!(
                    "XAI API returned an empty response body. Raw response saved to {}",
                    path.display()
                ),
                Err(dump_err) => anyhow::anyhow!(
                    "XAI API returned an empty response body. Also failed to write raw response to tmp: {}",
                    dump_err
                ),
            });
        }

        let output: SparkResponse = match serde_json::from_str(&content) {
            Ok(it) => it,
            Err(err) => {
                let dump_path = util::dump_raw_response_to_tmp(
                    &raw_response_text,
                    "dreamscroll-spark-response",
                );
                return Err(match dump_path {
                    Ok(path) => anyhow::anyhow!(
                        "Failed to parse structured spark response: {}. Full raw response saved to {}",
                        err,
                        path.display()
                    ),
                    Err(dump_err) => anyhow::anyhow!(
                        "Failed to parse structured spark response: {}. Also failed to write full raw response to tmp: {}",
                        err,
                        dump_err
                    ),
                });
            }
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
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
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
    let annotations = raw_response
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
                        content.iter().find_map(|part| {
                            let is_output_text =
                                part.get("type").and_then(|v| v.as_str()) == Some("output_text");
                            if is_output_text {
                                part.get("annotations").cloned()
                            } else {
                                None
                            }
                        })
                    })
            })
        })
        .or_else(|| {
            raw_response
                .get("choices")
                .and_then(|choices| choices.as_array())
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|m| m.get("annotations"))
                .cloned()
        });
    let citations = raw_response.get("citations").cloned().or_else(|| {
        raw_response
            .get("choices")
            .and_then(|choices| choices.as_array())
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|m| m.get("citations"))
            .cloned()
    });
    let sources = raw_response.get("sources").cloned();

    let num_sources_used = raw_response
        .get("usage")
        .and_then(|usage| usage.get("num_sources_used"))
        .cloned();

    let web_search_calls = raw_response
        .get("usage")
        .and_then(|usage| usage.get("server_side_tool_usage_details"))
        .and_then(|details| {
            details.get("web_search_calls").or_else(|| {
                details.get(1).or_else(|| {
                    details
                        .as_array()
                        .and_then(|arr| arr.iter().find(|it| it.get("web_search_calls").is_some()))
                })
            })
        })
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
                        content.iter().find_map(|part| {
                            let is_output_text =
                                part.get("type").and_then(|v| v.as_str()) == Some("output_text");
                            if is_output_text {
                                part.get("text").and_then(|v| v.as_str())
                            } else {
                                None
                            }
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
