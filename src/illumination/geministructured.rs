//! Gemini Structured Illuminator
//!
//! This module provides a Gemini-based illuminator that uses structured output
//! to return well-defined JSON responses instead of freeform text.
//!
//! ## Structured Output
//!
//! The Gemini API supports structured outputs via JSON Schema. By setting:
//! - `generationConfig.responseMimeType` to `"application/json"`
//! - `generationConfig.responseSchema` to a valid OpenAPI 3.0-style schema
//!
//! The model will return a response that strictly conforms to the schema.
//!
//! ## Response Structure
//!
//! The structured response contains:
//! - `summary`: A concise 1-2 sentence summary (max ~240 chars)
//! - `details`: A more detailed multi-paragraph description
//! - `suggested_searches`: A list of search queries to learn more
//!
//! ## Future Work
//!
//! Eventually the Illumination database model will be updated to store these
//! structured fields directly instead of a single freeform text blob.

use std::{env, io::Read, path::PathBuf};

use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api;

use super::Illuminator;

const PROMPT: &str = r#"
You are a virtual research assistant helping me to explore my world by analyzing
screenshots and other images I capture. You are my expert friend and guide. You
are someone I want to take with me to coffee shops, dive bars, late night movies,
and museum exhibits. You do not gush or flatter, but spark my interest and
inspiration.

I am sharing each image because I'm curious. I want to learn more and possibly take
action based on what I see. By analyzing each image, you will help me live a richer,
more informed life. Be engaging and insightful, not overly dry, verbose, or
clinical. Speak colorfully to inspire, and even humor me when appropriate. You are
not describing for a machine, but for a person; assume the reader can see the image
while reading your description. 

Analyze the attached image and provide your response in the structured JSON format
specified. 

For the summary: Provide a concise summary suitable for showing in a list with other
summaries, perhaps 2-3 sentences. This summary should provide crucial insights and
helpful details but not exceed 280 characters in length. Prioritize clarity and
concision. Help me understand the image's content and context, and empower me to learn
and discover new things. Do not describe obvious or mundane visual details from the
image like "the cover of book X has red letters and a white background", just say "X".
Don't say "This is a photograph showing X", just say "X". Don't say "An article snippet
from X...", just say "From X...". Don't say "A social media post from X...", just say
"X posts...". If a social media user X posts about Y, the summary should focus more
on Y. The focus should be on the underlying substance, not the format or medium.
Useful insights are preferred over a mere list of objects.

For the details: Give a more detailed description which can span several paragraphs.
Explore the content, context, and significance of what you see. Inform and empower me
to learn more and possibly take action. Imagine that I am viewing the details
alongside both the image and the concise summary. You can assume that I am viewing
the details because I was "hooked" by the summary and I want to learn more.

For suggested_searches: Provide a list of notable objects, people, or locations
visible in the image that merit follow-up. If the image features a montage of movies,
books, or articles, be sure to include suggestions for each one you can identify. Each
item should be a concise, helpful search query I can use to learn more about that
aspect of the image content. Ensure the queries are concise and natural. For example,
don't say "Stanley Kubrick and Andrei Tarkovsky relationship", just say "kubric, 
tarkovsky".
"#;

/// The structured response from the Gemini API for image illumination.
///
/// This represents the schema we request from Gemini's structured output feature.
/// The response is guaranteed to be valid JSON conforming to this structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredIllumination {
    /// A concise 1-2 sentence summary of the image content (max ~240 chars).
    /// Suitable for display in a list view alongside other summaries.
    pub summary: String,

    /// A detailed multi-paragraph description of the image content.
    /// Explores the content, context, and significance of the image.
    pub details: String,

    /// A list of suggested search queries to learn more about the image content.
    /// Each entry is a concise search query for a notable object, person, or
    /// location visible in the image.
    pub suggested_searches: Vec<String>,
}

impl StructuredIllumination {
    /// Converts the structured illumination back to a legacy freeform text format.
    ///
    /// This is useful for backwards compatibility with the current Illumination
    /// database model which stores a single text blob.
    ///
    /// Format:
    /// ```text
    /// <summary>
    ///
    /// <details>
    ///
    /// Suggested searches:
    /// - <search 1>
    /// - <search 2>
    /// ...
    /// ```
    pub fn to_legacy_text(&self) -> String {
        let mut result = format!("{}\n\n{}", self.summary, self.details);

        if !self.suggested_searches.is_empty() {
            result.push_str("\n\nSuggested searches:");
            for search in &self.suggested_searches {
                result.push_str(&format!("\n- {}", search));
            }
        }

        result
    }
}

/// Gemini-based illuminator that returns structured JSON responses.
///
/// Uses the Gemini API's structured output feature to guarantee responses
/// conform to a well-defined schema, enabling reliable parsing and storage
/// of individual response fields.
#[derive(Clone)]
pub struct GeminiStructuredIlluminator {
    api_key: String,
}

impl Default for GeminiStructuredIlluminator {
    fn default() -> Self {
        let api_key = env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY not found in env.");

        GeminiStructuredIlluminator { api_key }
    }
}

impl GeminiStructuredIlluminator {
    /// Illuminates a capture and returns the structured response directly.
    ///
    /// This method provides access to the parsed structured data, unlike the
    /// `Illuminator::illuminate` trait method which returns a legacy text format.
    pub async fn illuminate_structured(
        &self,
        capture: api::CaptureInfo,
    ) -> anyhow::Result<StructuredIllumination> {
        tracing::info!(
            "GeminiStructuredIlluminator: Illuminating capture ID {}",
            capture.id
        );

        let media1 = capture.medias.get(0).expect("No media found for capture.");
        let media1_path = PathBuf::from(format!("localdev/media/{}", &media1.filename));
        tracing::info!(
            "GeminiStructuredIlluminator: Using media at path {:?}",
            media1_path
        );

        let mut file = std::fs::File::open(media1_path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let enc = base64::engine::general_purpose::STANDARD.encode(&buffer);
        tracing::info!(
            "GeminiStructuredIlluminator: media base64 bytes {}",
            enc.len()
        );

        let client = Client::new();

        // Build the JSON schema for the structured response
        // Following the OpenAPI 3.0 schema format that Gemini expects
        let response_schema = json!({
            "type": "OBJECT",
            "properties": {
                "summary": {
                    "type": "STRING",
                    "description": "A concise 1-2 sentence summary of the image content, max 240 characters. Focus on substance, not format."
                },
                "details": {
                    "type": "STRING",
                    "description": "A detailed multi-paragraph description exploring the content, context, and significance of the image."
                },
                "suggested_searches": {
                    "type": "ARRAY",
                    "description": "A list of concise search queries for notable objects, people, or locations visible in the image.",
                    "items": {
                        "type": "STRING"
                    }
                }
            },
            "required": ["summary", "details", "suggested_searches"]
        });

        // Prepare request with text + image and structured output config
        let request_body = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {
                        "text": PROMPT
                    },
                    {
                        "inlineData": {
                            "mimeType": "image/jpeg",
                            "data": enc
                        }
                    }
                ]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": response_schema
            }
        });

        // Gemini endpoint with model in URL
        let model = "gemini-3-flash-preview";
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

            // Extract the JSON text from the response
            let json_text = &parsed_response.candidates[0].content.parts[0].text;

            // Parse the structured response
            let structured: StructuredIllumination = serde_json::from_str(json_text)
                .map_err(|e| anyhow::anyhow!("Failed to parse structured response: {}", e))?;

            tracing::info!(
                "GeminiStructuredIlluminator: Successfully parsed structured response with {} suggested searches",
                structured.suggested_searches.len()
            );

            Ok(structured)
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

#[async_trait::async_trait]
impl Illuminator for GeminiStructuredIlluminator {
    fn model_name(&self) -> &'static str {
        "gemini-structured"
    }

    async fn illuminate(&self, capture: api::CaptureInfo) -> anyhow::Result<String> {
        // Use the structured illumination and convert to legacy text format
        let structured = self.illuminate_structured(capture).await?;
        Ok(structured.to_legacy_text())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_illumination_to_legacy_text() {
        let illumination = StructuredIllumination {
            summary: "A fascinating book about quantum physics.".to_string(),
            details: "This appears to be a cover of a popular science book.\n\nThe author is well-known for making complex topics accessible.".to_string(),
            suggested_searches: vec![
                "quantum physics introduction".to_string(),
                "author name books".to_string(),
            ],
        };

        let legacy = illumination.to_legacy_text();

        assert!(legacy.contains("A fascinating book about quantum physics."));
        assert!(legacy.contains("This appears to be a cover"));
        assert!(legacy.contains("Suggested searches:"));
        assert!(legacy.contains("- quantum physics introduction"));
        assert!(legacy.contains("- author name books"));
    }

    #[test]
    fn test_structured_illumination_to_legacy_text_no_searches() {
        let illumination = StructuredIllumination {
            summary: "A simple image.".to_string(),
            details: "Nothing remarkable here.".to_string(),
            suggested_searches: vec![],
        };

        let legacy = illumination.to_legacy_text();

        assert!(legacy.contains("A simple image."));
        assert!(legacy.contains("Nothing remarkable here."));
        assert!(!legacy.contains("Suggested searches:"));
    }

    #[test]
    fn test_structured_illumination_deserialize() {
        let json = r#"{
            "summary": "Test summary",
            "details": "Test details",
            "suggested_searches": ["search1", "search2"]
        }"#;

        let parsed: StructuredIllumination = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.summary, "Test summary");
        assert_eq!(parsed.details, "Test details");
        assert_eq!(parsed.suggested_searches.len(), 2);
        assert_eq!(parsed.suggested_searches[0], "search1");
    }

    #[test]
    fn test_structured_illumination_serialize() {
        let illumination = StructuredIllumination {
            summary: "Test".to_string(),
            details: "Details".to_string(),
            suggested_searches: vec!["search".to_string()],
        };

        let json = serde_json::to_string(&illumination).unwrap();

        assert!(json.contains("\"summary\":\"Test\""));
        assert!(json.contains("\"details\":\"Details\""));
        assert!(json.contains("\"suggested_searches\":[\"search\"]"));
    }
}
