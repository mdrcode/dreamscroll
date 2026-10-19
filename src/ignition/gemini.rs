use reqwest::Client;
use serde::Serialize;
use serde_json::json;

use super::*;
use crate::llms::{InferenceMetadata, InferenceOutcome};

// Unfortunately, the Gemini API continues to hallucinate recommended links at
// an unacceptable rate, producing fabricated links and soft-404/homepage
// fallbacks. Future hardening idea: have the model emit grounding chunk
// references instead of free-form URLs, then resolve URLs from
// groundingMetadata server-side. This prevents fabricated links and
// soft-404/homepage fallbacks from passing as "valid" recommendations.

pub struct GeminiFirestarter {
    api_key: String,
    model_id: String,
}

impl GeminiFirestarter {
    pub fn new(api_key: String) -> Self {
        let model_id = "gemini-3-flash-preview".to_string();
        tracing::info!(model_id, "GeminiFirestarter initialized");
        Self { api_key, model_id }
    }
}

#[async_trait::async_trait]
impl Firestarter for GeminiFirestarter {
    fn name(&self) -> &str {
        "GeminiFirestarter"
    }

    async fn spark(
        &self,
        captures: Vec<crate::api::CaptureInfo>,
    ) -> anyhow::Result<InferenceOutcome<SparkResponse>> {
        let input_capture_count = captures.len();
        let user_prompt = util::append_captures_to_user_prompt(prompt::PROMPT, captures);

        let request_body = GenerateContentRequest {
            contents: vec![GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart { text: user_prompt }],
            }],
            // Gemini grounding docs: https://ai.google.dev/gemini-api/docs/google-search
            tools: vec![GeminiTool {
                google_search: GeminiGoogleSearch::default(),
            }],
            generation_config: GeminiGenerationConfig {
                response_mime_type: "application/json".to_string(),
                response_schema: spark_output_schema_gemini(),
            },
        };

        let model_url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model_id
        );

        let client = Client::new();
        let started = std::time::Instant::now();
        let response = client
            .post(&model_url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        let status = response.status();
        let raw_response = response.text().await?;
        let duration_ms = started.elapsed().as_millis() as i64;
        let response_json = serde_json::from_str(&raw_response).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse Gemini API response JSON: {}. Response excerpt: {}",
                err,
                util::truncate_for_error(&raw_response, 600)
            )
        })?;

        Ok(parse_gemini_response(
            response_json,
            status,
            &self.model_id,
            duration_ms,
            input_capture_count,
        ))
    }
}

fn parse_gemini_response(
    mut response: serde_json::Value,
    status: reqwest::StatusCode,
    model_id: &str,
    duration_ms: i64,
    input_capture_count: usize,
) -> InferenceOutcome<SparkResponse> {
    let mut metadata = InferenceMetadata {
        provider_name: "gemini".to_string(),
        backend_name: "developer_api".to_string(),
        model_id: model_id.to_string(),
        duration_ms,
        ..Default::default()
    };
    metadata.params.insert(
        "input_capture_count".to_string(),
        json!(input_capture_count),
    );
    if let Some(usage) = response
        .get_mut("usageMetadata")
        .map(serde_json::Value::take)
        .filter(|value| value.is_object())
    {
        metadata.tokens_in = usage
            .get("promptTokenCount")
            .and_then(serde_json::Value::as_i64);
        metadata.tokens_in_cached = usage
            .get("cachedContentTokenCount")
            .and_then(serde_json::Value::as_i64);
        metadata.tokens_out = usage
            .get("candidatesTokenCount")
            .and_then(serde_json::Value::as_i64);
        metadata.params.insert("provider_usage".to_string(), usage);
    }
    if let Some(id) = response
        .get("responseId")
        .and_then(serde_json::Value::as_str)
    {
        metadata
            .params
            .insert("provider_request_id".to_string(), json!(id));
    }
    if let Some(candidates) = response
        .get("candidates")
        .and_then(serde_json::Value::as_array)
    {
        let grounding = candidates
            .iter()
            .filter_map(|candidate| candidate.get("groundingMetadata"))
            .filter(|value| !value.is_null())
            .cloned()
            .collect::<Vec<_>>();
        if !grounding.is_empty() {
            metadata
                .params
                .insert("provider_grounding".to_string(), json!(grounding));
        }
    }

    let output = (|| {
        if !status.is_success() {
            anyhow::bail!("Gemini API error: {}", response);
        }
        let content = response
            .get("candidates")
            .and_then(serde_json::Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(|content| content.get("parts"))
            .and_then(serde_json::Value::as_array)
            .and_then(|parts| {
                parts
                    .iter()
                    .find_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            })
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            anyhow::bail!("Gemini API returned an empty structured response");
        }
        serde_json::from_str(content).map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse Gemini structured spark response: {}. Content excerpt: {}",
                err,
                util::truncate_for_error(content, 600)
            )
        })
    })();

    InferenceOutcome { output, metadata }
}

fn spark_output_schema_gemini() -> serde_json::Value {
    json!({
        "type": "OBJECT",
        "required": ["clusters"],
        "properties": {
            "clusters": {
                "type": "ARRAY",
                "items": {
                    "type": "OBJECT",
                    "required": ["title", "summary", "capture_ids", "recommended_links"],
                    "properties": {
                        "title": { "type": "STRING" },
                        "summary": { "type": "STRING" },
                        "capture_ids": {
                            "type": "ARRAY",
                            "items": { "type": "INTEGER" }
                        },
                        "recommended_links": {
                            "type": "ARRAY",
                            "items": {
                                "type": "OBJECT",
                                "required": ["url", "commentary"],
                                "properties": {
                                    "url": { "type": "STRING" },
                                    "commentary": { "type": "STRING" }
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
#[serde(rename_all = "camelCase")]
struct GenerateContentRequest {
    contents: Vec<GeminiContent>,
    tools: Vec<GeminiTool>,
    generation_config: GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiTool {
    google_search: GeminiGoogleSearch,
}

#[derive(Serialize, Default)]
struct GeminiGoogleSearch {}

#[derive(Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    response_mime_type: String,
    response_schema: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(response: serde_json::Value) -> InferenceOutcome<SparkResponse> {
        parse_gemini_response(
            response,
            reqwest::StatusCode::OK,
            "gemini-3-flash-preview",
            123,
            4,
        )
    }

    #[test]
    fn normalizes_direct_counts_and_preserves_raw_usage_and_grounding() {
        let usage = json!({
            "promptTokenCount": 5_000_000_000_i64,
            "cachedContentTokenCount": 4_000_000_000_i64,
            "candidatesTokenCount": 42,
            "totalTokenCount": 6_000_000_000_i64,
            "thoughtsTokenCount": 17,
            "toolUsePromptTokenCount": 19
        });
        let grounding = json!({"groundingChunks": [{"web": {"uri": "https://example.com"}}]});
        let outcome = parse(json!({
            "modelVersion": "different-response-model",
            "responseId": "gemini-request",
            "usageMetadata": usage,
            "candidates": [{
                "content": {"parts": [{"text": "{\"clusters\":[]}"}]},
                "groundingMetadata": grounding
            }]
        }));

        assert!(outcome.output.unwrap().clusters.is_empty());
        let metadata = outcome.metadata;
        assert_eq!(metadata.provider_name, "gemini");
        assert_eq!(metadata.backend_name, "developer_api");
        assert_eq!(metadata.model_id, "gemini-3-flash-preview");
        assert_eq!(metadata.duration_ms, 123);
        assert_eq!(metadata.tokens_in, Some(5_000_000_000));
        assert_eq!(metadata.tokens_in_cached, Some(4_000_000_000));
        assert_eq!(metadata.tokens_out, Some(42));
        assert_eq!(metadata.params["input_capture_count"], 4);
        assert_eq!(metadata.params["provider_request_id"], "gemini-request");
        assert_eq!(metadata.params["provider_usage"], usage);
        assert_eq!(metadata.params["provider_grounding"], json!([grounding]));
    }

    #[test]
    fn missing_and_invalid_counts_remain_null_without_total_arithmetic() {
        for response in [
            json!({}),
            json!({"usageMetadata": {"totalTokenCount": 99}}),
            json!({"usageMetadata": {
                "promptTokenCount": "12",
                "cachedContentTokenCount": null,
                "candidatesTokenCount": 1.5,
                "totalTokenCount": 99
            }}),
        ] {
            let metadata = parse(response).metadata;
            assert_eq!(metadata.tokens_in, None);
            assert_eq!(metadata.tokens_in_cached, None);
            assert_eq!(metadata.tokens_out, None);
        }
    }

    #[test]
    fn usage_survives_missing_candidates_and_output_parse_errors() {
        for mut response in [
            json!({}),
            json!({"candidates": [{"finishReason": "SAFETY"}]}),
            json!({"candidates": [{"content": {"parts": [{"text": ""}]}}]}),
            json!({"candidates": [{"content": {"parts": [{"text": "not json"}]}}]}),
            json!({"candidates": [{"content": {"parts": [{"text": "{}"}]}}]}),
        ] {
            let usage = json!({"promptTokenCount": 20, "cachedContentTokenCount": 8});
            response["usageMetadata"] = usage.clone();
            let outcome = parse(response);
            assert!(outcome.output.is_err());
            assert_eq!(outcome.metadata.tokens_in, Some(20));
            assert_eq!(outcome.metadata.tokens_in_cached, Some(8));
            assert_eq!(outcome.metadata.tokens_out, None);
            assert_eq!(outcome.metadata.params["provider_usage"], usage);
        }
    }

    #[test]
    fn http_error_preserves_usage_and_grounding_without_candidate_content() {
        let grounding = json!({"searchEntryPoint": {"renderedContent": "source"}});
        let outcome = parse_gemini_response(
            json!({
                "error": {"message": "failed"},
                "usageMetadata": {"promptTokenCount": 7},
                "candidates": [{"groundingMetadata": grounding}]
            }),
            reqwest::StatusCode::BAD_REQUEST,
            "gemini-3-flash-preview",
            12,
            1,
        );
        assert!(
            outcome
                .output
                .unwrap_err()
                .to_string()
                .contains("Gemini API error")
        );
        assert_eq!(outcome.metadata.tokens_in, Some(7));
        assert_eq!(
            outcome.metadata.params["provider_grounding"],
            json!([grounding])
        );
    }
}
