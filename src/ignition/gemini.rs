use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;

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

    async fn spark(&self, captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<SparkResult> {
        let started = std::time::Instant::now();
        let input_capture_count = captures.len() as i32;
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
        let response = client
            .post(&model_url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Gemini API error: {}", error_text);
        }

        let raw_response = response.text().await?;
        let parsed: GenerateContentResponse =
            serde_json::from_str(&raw_response).map_err(|err| {
                anyhow::anyhow!(
                    "Failed to parse Gemini API response JSON: {}. Response excerpt: {}",
                    err,
                    util::truncate_for_error(&raw_response, 600)
                )
            })?;
        let content = parsed
            .candidates
            .first()
            .and_then(|candidate| {
                candidate
                    .content
                    .parts
                    .iter()
                    .find_map(|part| part.text.as_deref())
            })
            .unwrap_or("")
            .trim()
            .to_string();

        if content.is_empty() {
            anyhow::bail!("Gemini API returned an empty structured response");
        }

        let output: SparkResponse = match serde_json::from_str(&content) {
            Ok(it) => it,
            Err(err) => anyhow::bail!(
                "Failed to parse Gemini structured spark response: {}. Content excerpt: {}",
                err,
                util::truncate_for_error(&content, 600)
            ),
        };

        let duration_ms = started.elapsed().as_millis() as i64;
        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_gemini_usage(parsed.usage_metadata);

        let provider_grounding_json = parse_gemini_grounding(&parsed.candidates);

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

fn parse_gemini_usage(
    usage: Option<serde_json::Value>,
) -> (Option<i32>, Option<i32>, Option<i32>, Option<String>) {
    let Some(usage) = usage else {
        return (None, None, None, None);
    };

    let input_tokens = usage
        .get("promptTokenCount")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let output_tokens = usage
        .get("candidatesTokenCount")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let total_tokens = usage
        .get("totalTokenCount")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let usage_json = serde_json::to_string(&usage).ok();
    (input_tokens, output_tokens, total_tokens, usage_json)
}

fn parse_gemini_grounding(candidates: &[GeminiCandidate]) -> Option<String> {
    let grounding = candidates
        .iter()
        .filter_map(|candidate| candidate.grounding_metadata.clone())
        .collect::<Vec<_>>();

    if grounding.is_empty() {
        None
    } else {
        serde_json::to_string(&grounding).ok()
    }
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateContentResponse {
    candidates: Vec<GeminiCandidate>,
    usage_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: GeminiResponseContent,
    grounding_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: Option<String>,
}
