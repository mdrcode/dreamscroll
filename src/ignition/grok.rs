use reqwest::Client;
use serde::Serialize;
use serde_json::json;

use super::*;

pub struct GrokFirestarter {
    api_key: String,
    model_id: &'static str,
    client: Client,
}

impl GrokFirestarter {
    pub fn new(api_key: String) -> Self {
        let model_id = "grok-4-1-fast-reasoning";
        tracing::info!(model_id, "GrokFirestarter initialized");
        Self {
            api_key,
            model_id,
            client: Client::new(),
        }
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
            model: self.model_id,
            input: vec![InputMessage {
                role: "user",
                content: user_prompt,
            }],
            // xAI Agent Tools docs: https://docs.x.ai/developers/tools/overview
            tools: vec![GrokTool::WebSearch],
            text: ResponseText {
                format: ResponseTextFormat {
                    format_type: "json_schema",
                    name: "spark_output",
                    strict: true,
                    schema: spark_output_schema_grok(),
                },
            },
        };

        let started = std::time::Instant::now();
        let response = self
            .client
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
        let output_text_part = find_grok_output_text_part(&response_json);
        let output_text = output_text_part
            .and_then(|part| part.get("text"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());

        let Some(output_text) = output_text else {
            let response_trace = response_json.to_string();
            anyhow::bail!(
                "XAI API response did not contain valid output text. Excerpt: {}",
                util::truncate_for_error(&response_trace, 600)
            );
        };

        let spark = serde_json::from_str::<SparkResponse>(output_text).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse structured spark response: {}. Excerpt: {}",
                err,
                util::truncate_for_error(output_text, 600)
            )
        })?;

        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_grok_usage(&response_json);

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
                provider_grounding_json: None,
            },
        })
    }
}

fn find_grok_output_text_part(response_json: &serde_json::Value) -> Option<&serde_json::Value> {
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
                        content.iter().find(|part| {
                            part.get("type").and_then(serde_json::Value::as_str)
                                == Some("output_text")
                        })
                    })
            })
        })
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
    model: &'static str,
    input: Vec<InputMessage>,
    tools: Vec<GrokTool>,
    text: ResponseText,
}

#[derive(Serialize)]
struct InputMessage {
    role: &'static str,
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
    format_type: &'static str,
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}
