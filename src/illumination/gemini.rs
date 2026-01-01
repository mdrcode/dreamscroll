use std::{env, io::Read, path::PathBuf};

use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::Illuminator;
use crate::controller;

#[derive(Clone)]
pub struct GeminiIlluminator {
    api_key: String,
}

impl Default for GeminiIlluminator {
    fn default() -> Self {
        let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not found in env.");

        GeminiIlluminator { api_key }
    }
}

#[async_trait::async_trait]
impl Illuminator for GeminiIlluminator {
    async fn illuminate(&self, capture: controller::CaptureInfo) -> anyhow::Result<String> {
        tracing::info!("GeminiIlluminator: Illuminating capture ID {}", capture.id);

        let media1 = capture.medias.get(0).expect("No media found for capture.");
        let media1_path = PathBuf::from(format!("localdev/media/{}", &media1.filename));
        tracing::info!("GeminiIlluminator: Using media at path {:?}", media1_path);

        let mut file = std::fs::File::open(media1_path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let enc = base64::engine::general_purpose::STANDARD.encode(&buffer);
        tracing::info!(
            "GeminiIlluminator: Encoded media base64 bytes {}",
            enc.len()
        );

        // Create client
        let client = Client::new();

        // Prepare request with text + image (no separate system role; bake into user)
        let request_body = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![
                    Part::Text {
                        text: "Describe the content of this image in detail. If you recognize the image, please identify it.".to_string(),
                    },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type: "image/jpeg".to_string(), // Adjust if PNG: "image/png"
                            data: enc,
                        },
                    },
                ],
            }],
        };

        // Gemini endpoint with model in URL
        let model = "gemini-3-flash-preview"; // Use a vision model; check docs for latest
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            model
        );

        // Send request
        let response = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if response.status().is_success() {
            let parsed_response: GenerateContentResponse = response.json().await?;
            let r = parsed_response.candidates[0].content.parts[0].text.clone();
            Ok(r)
        } else {
            let status_code = response.status();
            let error_text = response.text().await?;
            Err(anyhow::anyhow!(
                "API Error status {}: {}",
                status_code,
                error_text
            ))?
        }
    }
}

#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
}

#[derive(Serialize)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Text { text: String },
    InlineData { inline_data: InlineData },
}

#[derive(Serialize)]
struct InlineData {
    mime_type: String,
    data: String, // Pure base64, no data URI prefix
}

// Response structures for Gemini (minimal)
#[derive(Deserialize, Debug)]
struct GenerateContentResponse {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize, Debug)]
struct Candidate {
    content: ContentResponse,
}

#[derive(Deserialize, Debug)]
struct ContentResponse {
    parts: Vec<PartResponse>,
}

#[derive(Deserialize, Debug)]
struct PartResponse {
    text: String,
}
