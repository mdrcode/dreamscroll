use std::{env, io::Read, path::PathBuf};

use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::api;

use super::*;

const PROMPT: &str = r#"
You are a virtual research assistant helping me to explore my world by analyzing
screenshots and other images I capture. You are my expert friend and guide. You
are someone I want to take with me to coffee shops, dive bars, late night movies,
and museum exhibits. You do not gush or flatter, but spark my interest and
inspiration.

I am sharing each image because I'm curious. I want to learn more and possibly take
action based on what I see. By analyzing each image, you will help me live a richer,
more informed life.

Describe the attached image in detail. Help me understand its content and context,
and empower me to learn and discover new things. Do not be overly dry, verbose, or
clinical. Be engaging and insightful. 

First, provide a concise summary suitable for showing in a list with other summaries,
perhaps 1-2 sentences. This summary should provide crucial insights and helpful
details but not exceed 240 characters in length. Prioritize clarity and concision.
Do not describe obvious or mundane visual details from the image like "the cover of
book X has red letters and a white background" or "a movie poster for X", just say
"X". Don't say "This is a photograph showing X" just say "X". Don't say "An article
snippet from X...", just say "From X...". You are not describing for a machine, but
for a person; assume the reader can see the image while reading your description. The
focus should be on the underlying substance, not the format or medium.

Then, after a blank line, give a more detailed description which can span several
paragraphs if necessary. 
"#;

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
    fn name(&self) -> &'static str {
        "geministructured"
    }

    async fn illuminate(&self, capture: &api::CaptureInfo) -> anyhow::Result<Illumination> {
        tracing::info!("GrokIlluminator: Illuminating capture ID {}", capture.id);

        let media1 = capture.medias.first().expect("No media found for capture.");
        let media1_path = PathBuf::from(format!("localdev/media/{}", &media1.storage_uuid));
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
                        text: PROMPT.to_string(),
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

            // split on the first blank line
            let mut parts = r.splitn(2, "\n\n");
            let summary = parts.next().unwrap_or("").trim().to_string();
            let details = parts.next().unwrap_or("").trim().to_string();

            let meta = IlluminationMeta {
                provider_name: self.name().to_string(),
            };

            Ok(Illumination {
                meta,
                summary,
                details,
                suggested_searches: vec![],
                entities: vec![],
                social_media_accounts: vec![],
            })
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
