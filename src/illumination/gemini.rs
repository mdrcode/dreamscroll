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

Finally, after another blank line, provide a bullet point list of notable objects,
people, or locations visible in the image. Please pay special attention to objects
which merit followup, like a picture of a book I can read, or a movie I can watch.
Feel free to suggest a concise, helpful (but not too verbose) search query suggestion
I can use to learn more about the image content, by ending the line item with 
"(search: ... )".
"#;

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
    fn name(&self) -> &'static str {
        "geministructured"
    }

    async fn illuminate(&self, capture: api::CaptureInfo) -> anyhow::Result<Illumination> {
        tracing::info!("GeminiIlluminator: Illuminating capture ID {}", capture.id);

        let media1 = capture.medias.get(0).expect("No media found for capture.");
        let media1_path = PathBuf::from(format!("localdev/media/{}", &media1.filename));
        tracing::info!("GeminiIlluminator: Using media at path {:?}", media1_path);

        let mut file = std::fs::File::open(media1_path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let enc = base64::engine::general_purpose::STANDARD.encode(&buffer);
        tracing::info!("GeminiIlluminator: media base64 bytes {}", enc.len());

        let client = Client::new();

        // Prepare request with text + image (no separate system role; bake into user)
        let request_body = GenerateContentRequest {
            contents: vec![Content {
                role: "user".to_string(),
                parts: vec![
                    Part::Text {
                        text: PROMPT.to_string(),
                    },
                    Part::InlineData {
                        inline_data: InlineData {
                            mime_type: "image/jpeg".to_string(),
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

            // split on blank lines
            let mut sections = r.split("\n\n");
            let summary = sections.next().unwrap_or("").trim().to_string();
            let details = sections.next().unwrap_or("").trim().to_string();

            let meta = IlluminationMeta {
                provider_name: self.name().to_string(),
            };

            Ok(Illumination {
                meta,
                summary,
                details,
                suggested_searches: vec![],
                entities: vec![],
            })
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
