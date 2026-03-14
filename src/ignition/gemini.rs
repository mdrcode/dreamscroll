use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;

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
        let parsed: GenerateContentResponse = serde_json::from_str(&raw_response)?;
        let content = parsed
            .candidates
            .first()
            .and_then(|candidate| candidate.content.parts.first())
            .map(|part| part.text.trim())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            anyhow::bail!("Gemini API returned an empty structured response");
        }

        let output: SparkResponse = match serde_json::from_str(&content) {
            Ok(it) => it,
            Err(err) => {
                let dump_path =
                    util::dump_raw_response_to_tmp(&content, "dreamscroll-gemini-spark-response");
                return Err(match dump_path {
                    Ok(path) => anyhow::anyhow!(
                        "Failed to parse Gemini structured spark response: {}. Raw JSON saved to {}",
                        err,
                        path.display()
                    ),
                    Err(dump_err) => anyhow::anyhow!(
                        "Failed to parse Gemini structured spark response: {}. Also failed to write raw JSON to tmp: {}",
                        err,
                        dump_err
                    ),
                });
            }
        };

        let duration_ms = started.elapsed().as_millis() as i64;
        let (input_tokens, output_tokens, total_tokens, provider_usage_json) =
            parse_gemini_usage(parsed.usage_metadata);

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
    generation_config: GeminiGenerationConfig,
}

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
struct GeminiCandidate {
    content: GeminiResponseContent,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: String,
}
