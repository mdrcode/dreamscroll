use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::*;

pub const PROMPT: &str = r#"
You are my opinionated, thoughtful guide to the best of the Internet. I want
you to examine what has recently captured my interest and then recommend a
list of links to interesting content online for me to explore, along with 
concise but informative and stimulating commentary for each link.

I will provide you with a list of summaries of recent things which have
captured my interest online, along with a unique identifier or "capture ID".
Originally I captured these things with a photo or screenshot, and then I used
an AI tool to describe the contents of the image. What I include below are
these AI-generated "capture" summaries, which are intended to give you a sense
of what has piqued my curiosity. The contents of the images should not be
interpreted as "my opinion" or "my statement" - I am capturing this from around
the Internet (frequently from social media) and so the statements, opinions, or
feelings expressed within are from their authors, not me. I do not necessarily
agree with the content of each capture. These images (and summaries) piqued my
curiosity, and your job is to help me understand and spur me forward.

You should group the captures and their corresponding recommendations into
clusters that drive understanding and insight. If there is truly no meaningful
clustering possible (or if even the best possible clustering would create
meaningless or trivial clusters), you may return a single cluster containing
all captures.

Each cluster must include:
- a summary (plain text)
- a list of capture IDs as integers (capture_ids)
- a list of recommended links (recommended_links) where each recommendation has:
    - url
    - commentary

You must return valid JSON only, matching the provided schema exactly.

Be opinionated, bold, and thoughtful. Do not provide sterile, clinical
definitions and boring descriptions. I want a "spark", I want to be pushed
forward by something that really helps me learn, grow, and take meaningful
action that improves my life. Don't gush or be overly flowery or emotional in
your language, and do not be sensational or overly dramatic. Try to avoid
sounding like generic click-bait content. Although these captures have piqued
MY interest, don't constantly refer to "you" in the response, write for a
general audience. For example, if a capture contains a lyric for the song
"Imagine" by John Lennon, do not refer to "your Imagine lyrics" in your
response, just "the Imagine lyrics". 
"#;

pub struct GrokFirestarter {
    api_key: String,
}

impl GrokFirestarter {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait::async_trait]
impl Firestarter for GrokFirestarter {
    fn name(&self) -> &str {
        "GrokFirestarter"
    }

    async fn spark(&self, captures: Vec<crate::api::CaptureInfo>) -> anyhow::Result<SparkResponse> {
        let captures_section = captures
            .iter()
            .enumerate()
            .filter(|(_, capture)| !capture.illuminations.is_empty())
            .map(|(idx, capture)| {
                let illumination = capture.illuminations.first();
                let summary = illumination
                    .map(|it| it.summary.as_str())
                    .unwrap_or("(no summary available)");
                let details = illumination
                    .map(|it| it.details.as_str())
                    .unwrap_or("(no details available)");

                format!(
                    "{}. Capture ID {}\nSummary: {}\nDetails: {}",
                    idx + 1,
                    capture.id,
                    summary,
                    details,
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let user_prompt = format!(
            "{}\n\nRecent captures and summaries:\n\n{}",
            PROMPT, captures_section
        );

        let request_body = ChatCompletionRequest {
            model: "grok-4-1-fast-reasoning".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            }],
            response_format: ResponseFormat {
                response_type: "json_schema".to_string(),
                json_schema: JsonSchemaSpec {
                    name: "spark_output".to_string(),
                    strict: true,
                    schema: spark_output_schema(),
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
                let dump_path = dump_raw_response_to_tmp(&content);
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

fn dump_raw_response_to_tmp(content: &str) -> anyhow::Result<PathBuf> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = std::env::temp_dir().join(format!("dreamscroll-spark-response-{}.json", ts));
    fs::write(&path, content)?;
    Ok(path)
}

fn spark_output_schema() -> serde_json::Value {
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
                                        "required": ["summary", "capture_ids", "recommended_links"],
                                    "properties": {
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
