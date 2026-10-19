use reqwest::Client;
use serde::Serialize;
use serde_json::json;

use super::*;
use crate::llms::{InferenceMetadata, InferenceOutcome};

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

    async fn spark(
        &self,
        captures: Vec<crate::api::CaptureInfo>,
    ) -> anyhow::Result<InferenceOutcome<SparkResponse>> {
        let input_capture_count = captures.len();
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
        let status = response.status();
        let raw_response = response.text().await?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let response_json = serde_json::from_str(&raw_response)?;

        Ok(parse_grok_response(
            response_json,
            status,
            self.model_id,
            duration_ms,
            input_capture_count,
        ))
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

fn parse_grok_response(
    mut response: serde_json::Value,
    status: reqwest::StatusCode,
    model_id: &str,
    duration_ms: i64,
    input_capture_count: usize,
) -> InferenceOutcome<SparkResponse> {
    let mut metadata = InferenceMetadata {
        provider_name: "xai".to_string(),
        backend_name: "responses".to_string(),
        model_id: model_id.to_string(),
        duration_ms,
        ..Default::default()
    };
    metadata.params.insert(
        "input_capture_count".to_string(),
        json!(input_capture_count),
    );
    if let Some(usage) = response
        .get_mut("usage")
        .map(serde_json::Value::take)
        .filter(|value| value.is_object())
    {
        metadata.tokens_in = usage
            .get("input_tokens")
            .and_then(serde_json::Value::as_i64);
        metadata.tokens_in_cached = usage
            .get("input_tokens_details")
            .and_then(|details| details.get("cached_tokens"))
            .and_then(serde_json::Value::as_i64);
        metadata.tokens_out = usage
            .get("output_tokens")
            .and_then(serde_json::Value::as_i64);
        metadata.params.insert("provider_usage".to_string(), usage);
    }
    if let Some(id) = response.get("id").and_then(serde_json::Value::as_str) {
        metadata
            .params
            .insert("provider_request_id".to_string(), json!(id));
    }
    let mut grounding = serde_json::Map::new();
    if let Some(citations) = response.get("citations").filter(|value| !value.is_null()) {
        grounding.insert("citations".to_string(), citations.clone());
    }
    if let Some(output) = response.get("output").and_then(serde_json::Value::as_array) {
        let annotations = output
            .iter()
            .filter(|item| item.get("type").and_then(serde_json::Value::as_str) == Some("message"))
            .filter_map(|item| item.get("content").and_then(serde_json::Value::as_array))
            .flatten()
            .filter(|part| {
                part.get("type").and_then(serde_json::Value::as_str) == Some("output_text")
            })
            .filter_map(|part| {
                part.get("annotations")
                    .and_then(serde_json::Value::as_array)
            })
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        if !annotations.is_empty() {
            grounding.insert("annotations".to_string(), json!(annotations));
        }
    }
    if !grounding.is_empty() {
        metadata
            .params
            .insert("provider_grounding".to_string(), grounding.into());
    }

    let output = (|| {
        if !status.is_success() {
            anyhow::bail!("XAI API error: {}", response);
        }
        let output_text = find_grok_output_text_part(&response)
            .and_then(|part| part.get("text"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "XAI API response did not contain valid output text. Excerpt: {}",
                    util::truncate_for_error(&response.to_string(), 600)
                )
            })?;
        serde_json::from_str(output_text).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse structured spark response: {}. Excerpt: {}",
                err,
                util::truncate_for_error(output_text, 600)
            )
        })
    })();

    InferenceOutcome { output, metadata }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(response: serde_json::Value) -> InferenceOutcome<SparkResponse> {
        parse_grok_response(
            response,
            reqwest::StatusCode::OK,
            "grok-4-1-fast-reasoning",
            123,
            4,
        )
    }

    #[test]
    fn normalizes_direct_counts_and_preserves_usage_and_grounding() {
        let usage = json!({
            "input_tokens": 5_000_000_000_i64,
            "input_tokens_details": {"cached_tokens": 4_000_000_000_i64},
            "output_tokens": 42,
            "output_tokens_details": {"reasoning_tokens": 17},
            "total_tokens": 6_000_000_000_i64
        });
        let annotation = json!({"type": "url_citation", "url": "https://example.com"});
        let citations = json!(["https://example.com", "https://other.example"]);
        let outcome = parse(json!({
            "id": "xai-request",
            "model": "different-response-model",
            "usage": usage,
            "citations": citations,
            "output": [{"type": "message", "content": [{
                "type": "output_text",
                "text": "{\"clusters\":[]}",
                "annotations": [annotation]
            }]}]
        }));

        assert!(outcome.output.unwrap().clusters.is_empty());
        let metadata = outcome.metadata;
        assert_eq!(metadata.provider_name, "xai");
        assert_eq!(metadata.backend_name, "responses");
        assert_eq!(metadata.model_id, "grok-4-1-fast-reasoning");
        assert_eq!(metadata.duration_ms, 123);
        assert_eq!(metadata.tokens_in, Some(5_000_000_000));
        assert_eq!(metadata.tokens_in_cached, Some(4_000_000_000));
        assert_eq!(metadata.tokens_out, Some(42));
        assert_eq!(metadata.params["input_capture_count"], 4);
        assert_eq!(metadata.params["provider_request_id"], "xai-request");
        assert_eq!(metadata.params["provider_usage"], usage);
        assert_eq!(
            metadata.params["provider_grounding"],
            json!({
                "citations": citations,
                "annotations": [annotation]
            })
        );
    }

    #[test]
    fn missing_and_invalid_counts_remain_null_without_total_arithmetic() {
        for response in [
            json!({}),
            json!({"usage": {"total_tokens": 99}}),
            json!({"usage": {
                "input_tokens": "12",
                "input_tokens_details": {"cached_tokens": null},
                "output_tokens": 1.5,
                "total_tokens": 99
            }}),
        ] {
            let metadata = parse(response).metadata;
            assert_eq!(metadata.tokens_in, None);
            assert_eq!(metadata.tokens_in_cached, None);
            assert_eq!(metadata.tokens_out, None);
        }
    }

    #[test]
    fn usage_survives_missing_text_and_output_parse_errors() {
        for mut response in [
            json!({}),
            json!({"output": [{"type": "web_search_call"}]}),
            json!({"output": [{"type": "message", "content": [{"type": "refusal"}]}]}),
            json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": ""}]}]}),
            json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": "not json"}]}]}),
            json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": "{}"}]}]}),
        ] {
            let usage = json!({"input_tokens": 20, "input_tokens_details": {"cached_tokens": 8}});
            response["usage"] = usage.clone();
            let outcome = parse(response);
            assert!(outcome.output.is_err());
            assert_eq!(outcome.metadata.tokens_in, Some(20));
            assert_eq!(outcome.metadata.tokens_in_cached, Some(8));
            assert_eq!(outcome.metadata.tokens_out, None);
            assert_eq!(outcome.metadata.params["provider_usage"], usage);
        }
    }

    #[test]
    fn http_error_preserves_usage_and_annotations_without_valid_text() {
        let annotations = json!([{"type": "url_citation", "url": "https://example.com"}]);
        let outcome = parse_grok_response(
            json!({
                "error": {"message": "failed"},
                "usage": {"input_tokens": 7},
                "output": [{"type": "message", "content": [{
                    "type": "output_text", "annotations": annotations
                }]}]
            }),
            reqwest::StatusCode::BAD_REQUEST,
            "grok-4-1-fast-reasoning",
            12,
            1,
        );
        assert!(
            outcome
                .output
                .unwrap_err()
                .to_string()
                .contains("XAI API error")
        );
        assert_eq!(outcome.metadata.tokens_in, Some(7));
        assert_eq!(
            outcome.metadata.params["provider_grounding"]["annotations"],
            annotations
        );
    }
}
