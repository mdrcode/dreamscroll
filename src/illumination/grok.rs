use std::{env, io::Read, path::PathBuf};

use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::Illuminator;
use crate::controller;

#[derive(Clone)]
pub struct GrokIlluminator {
    api_key: String,
}

impl Default for GrokIlluminator {
    fn default() -> Self {
        let api_key = env::var("XAI_API_KEY").expect("XAI_API_KEY not found in env.");

        GrokIlluminator { api_key }
    }
}

#[async_trait::async_trait]
impl Illuminator for GrokIlluminator {
    fn model_name(&self) -> &'static str {
        "grok"
    }

    async fn illuminate(&self, capture: controller::CaptureInfo) -> anyhow::Result<String> {
        tracing::info!("GrokIlluminator: Illuminating capture ID {}", capture.id);

        let media1 = capture.medias.get(0).expect("No media found for capture.");
        let media1_path = PathBuf::from(format!("localdev/media/{}", &media1.filename));
        tracing::info!("GrokIlluminator: Using media at path {:?}", media1_path);

        let mut file = std::fs::File::open(media1_path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let enc = base64::engine::general_purpose::STANDARD.encode(&buffer);
        tracing::info!("GrokIlluminator: media base64 bytes {}", enc.len());

        let client = Client::new();

        let request_body = ChatCompletionRequest {
            model: "grok-4-1-fast-reasoning".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Array(vec![
                    ContentItem::Text {
                        text: "Describe the content of this image in detail. If you recognize the image, please identify it.".to_string(),
                    },
                    ContentItem::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:image/jpeg;base64,{}", enc),
                            detail: Some("high".to_string()), // low, auto, high
                        },
                    },
                ]),
            }],
        };

        let start = std::time::Instant::now();
        let response = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;
        let duration = start.elapsed();
        println!(
            "GrokIlluminator: API request completed with status {} in {:?}",
            response.status(),
            duration
        ); // TODO 

        if response.status().is_success() {
            let parsed_response: ChatCompletionResponse = response.json().await?;
            let r = parsed_response.choices[0].message.content.clone();
            Ok(r)
        } else {
            let error_text = response.text().await?;
            Err(anyhow::anyhow!("API Error: {}", error_text))?
        }
    }
}

// Define the request structure (matches the API's JSON body)
#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    #[allow(unused)]
    Text(String),
    Array(Vec<ContentItem>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>, // "auto", "low", or "high"
}

#[derive(Deserialize, Debug)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
struct Choice {
    message: MessageResponse,
}

#[derive(Deserialize, Debug)]
struct MessageResponse {
    content: String,
}
