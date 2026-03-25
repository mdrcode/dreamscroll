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
        let input_capture_count = captures.len() as i32;
        let user_prompt = util::append_captures_to_user_prompt(prompt::PROMPT, captures);

        let request_body = GrokRequest {
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
        let started = std::time::Instant::now();
        let response = client
            .post("https://api.x.ai/v1/responses")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;
        let duration_ms = started.elapsed().as_millis() as i64;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("XAI API error: {}", error_text);
        }

        let response_json: serde_json::Value = response.json().await?;

        let Some(response_output) = extract_grok_output(&response_json) else {
            anyhow::bail!(
                "XAI API returned an empty or invalid response. Excerpt: {}",
                response_json
                    .to_string()
                    .chars()
                    .take(600)
                    .collect::<String>()
            );
        };

        let spark = serde_json::from_str::<SparkResponse>(response_output).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse structured spark response: {}. Content excerpt: {}",
                err,
                util::truncate_for_error(response_output, 600)
            )
        })?;
        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_grok_usage(&response_json);
        let provider_grounding_json = parse_grok_grounding(&response_json);

        Ok(SparkResult {
            spark,
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

fn extract_grok_output(response_json: &serde_json::Value) -> Option<&str> {
    response_json
        .get("output")
        .and_then(serde_json::Value::as_array)
        .and_then(|output| {
            output.iter().find_map(|item| {
                if item.get("type").and_then(serde_json::Value::as_str) != Some("message") {
                    return None;
                }

                item.get("content")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|content| {
                        content.iter().find_map(|part| {
                            if part.get("type").and_then(serde_json::Value::as_str)
                                != Some("output_text")
                            {
                                return None;
                            }

                            part.get("text").and_then(serde_json::Value::as_str)
                        })
                    })
            })
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn parse_grok_usage(
    response_json: &serde_json::Value,
) -> (Option<i32>, Option<i32>, Option<i32>, Option<String>) {
    let Some(usage) = response_json.get("usage") else {
        return (None, None, None, None);
    };

    let token_count = |field: &str| {
        usage
            .get(field)
            .and_then(serde_json::Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
    };

    (
        token_count("input_tokens"),
        token_count("output_tokens"),
        token_count("total_tokens"),
        Some(usage.to_string()),
    )
}

fn parse_grok_grounding(response_json: &serde_json::Value) -> Option<String> {
    let annotations = response_json
        .get("output")
        .and_then(serde_json::Value::as_array)
        .and_then(|output| {
            output.iter().find_map(|item| {
                if item.get("type").and_then(serde_json::Value::as_str) != Some("message") {
                    return None;
                }

                item.get("content")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|content| {
                        content.iter().find_map(|part| {
                            if part.get("type").and_then(serde_json::Value::as_str)
                                != Some("output_text")
                            {
                                return None;
                            }

                            part.get("annotations").cloned()
                        })
                    })
            })
        });

    let usage = response_json.get("usage")?;
    let num_sources_used = usage.get("num_sources_used").cloned();
    let web_search_calls = usage
        .get("server_side_tool_usage_details")
        .and_then(|details| details.get("web_search_calls"))
        .cloned();

    if annotations.is_none() && num_sources_used.is_none() && web_search_calls.is_none() {
        return None;
    }

    let grounding = serde_json::json!({
        "annotations": annotations,
        "num_sources_used": num_sources_used,
        "web_search_calls": web_search_calls,
    });

    Some(grounding.to_string())
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
struct GrokRequest {
    model: String,
    input: Vec<InputMessage>,
    tools: Vec<GrokTool>,
    text: ResponseText,
}

#[derive(Serialize)]
struct InputMessage {
    role: String,
    content: String,
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
