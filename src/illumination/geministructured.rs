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
//! - `entities`: A list of notable entities with descriptions and types (person, place, book, movie, television_show, etc. See `EntityType` enum for full list)
//!

use std::{env, io::Read, path::PathBuf};

use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

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
more informed life. Be engaging and insightful, not overly dry, verbose, or
clinical. Speak colorfully to inspire, and even humor me when appropriate. You are
not describing for a machine, but for a person; assume the reader can see the image
while reading your description. 

Analyze the attached image and provide your response in the structured JSON format
specified.  Help me understand its content and context, and empower me to learn and
discover new things. Do not be overly dry, verbose, or clinical. Be engaging and
insightful. 

For the summary: First, provide a concise summary suitable for showing in a list
with other summaries, perhaps 1-2 sentences. This summary should provide crucial
insights and helpful details but not exceed 240 characters in length. Prioritize
clarity and concision. Do not describe obvious or mundane visual details from the
image like "the cover of book X has red letters and a white background" or "a movie
poster for X", just say "X". Don't say "This is a photograph showing X" just say "X".
Don't say "An article snippet from X...", just say "From X...". You are not
describing for a machine, but for a person; assume the reader can see the image
while reading your description. The focus should be on the underlying substance, not
the format or medium.

For the details: Give a more detailed description which should span two paragraphs
or more. Explore the content, context, and significance of what you see. Inform and 
empower me to learn more and possibly take action. You can assume that I am viewing
the image at the same time. Imagine that I am viewing the details because I was
"hooked" by your summary and I want to learn more and possibly take follow up action.

For suggested_searches: Provide a list of notable objects, people, or locations
visible in the image that merit follow-up. If the image features a montage of movies,
books, or articles, be sure to include suggestions for each one you can identify. Each
item should be a concise, helpful search query I can use to learn more about that
aspect of the image content. Ensure the queries are concise and natural. For example,
don't say "Stanley Kubrick and Andrei Tarkovsky relationship", just say "kubrick 
tarkovsky".

For entities: Identify and list notable and recognizable objects, people, locations, 
and references visible in the image. For each entity, provide its name, a brief 
description, and classify its type. Focus on entities that are noteworthy, culturally 
significant, or would be interesting to learn more about. Examples: books (with title 
and author), movies, brands, landmarks, famous people, artwork, etc. Be concise but 
informative. Entity types should be one of: person, place, book, movie,
television,  painting, meme, software, financial, youtuber, brand, or
unknown (for entities that don't fit other categories).
"#;

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

#[async_trait::async_trait]
impl Illuminator for GeminiStructuredIlluminator {
    fn name(&self) -> &'static str {
        "geministructured"
    }

    /// Illuminates a capture and returns the structured response directly.
    async fn illuminate(&self, capture: api::CaptureInfo) -> anyhow::Result<Illumination> {
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
        let entity_type_enum: Vec<String> = {
            use strum::IntoEnumIterator;
            EntityType::iter().map(|e| e.as_ref().to_string()).collect()
        };
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
                },
                "entities": {
                    "type": "ARRAY",
                    "description": "A list of notable entities (objects, people, locations, references) with descriptions and types.",
                    "items": {
                        "type": "OBJECT",
                        "properties": {
                            "name": {
                                "type": "STRING",
                                "description": "The name of the entity"
                            },
                            "description": {
                                "type": "STRING",
                                "description": "A brief description of the entity"
                            },
                            "type": {
                                "type": "STRING",
                                "description": "The type of entity: person, place, book, movie, television_show, brand, or unknown",
                                "enum": entity_type_enum
                            }
                        },
                        "required": ["name", "description", "type"]
                    }
                }
            },
            "required": ["summary", "details", "suggested_searches", "entities"]
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
            let structured: Illumination = serde_json::from_str(json_text)
                .map_err(|e| anyhow::anyhow!("Failed to parse structured response: {}", e))?;

            tracing::info!(
                "GeminiStructuredIlluminator: Successfully parsed structured response with {} entities and {} suggested searches",
                structured.entities.len(),
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
        let illumination = Illumination {
            meta: IlluminationMeta { provider_name: "testilluminator".to_string() },
            summary: "A fascinating book about quantum physics.".to_string(),
            details: "This appears to be a cover of a popular science book.\n\nThe author is well-known for making complex topics accessible.".to_string(),
            entities: vec![
                Entity {
                    name: "Quantum Physics Book".to_string(),
                    description: "A popular science book on quantum mechanics".to_string(),
                    entity_type: EntityType::Book,
                },
            ],
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
        assert!(legacy.contains("Entities:"));
        assert!(legacy.contains(
            "- Quantum Physics Book [book]: A popular science book on quantum mechanics"
        ));

        // Verify entities come after suggested searches
        let searches_pos = legacy.find("Suggested searches:").unwrap();
        let entities_pos = legacy.find("Entities:").unwrap();
        assert!(
            entities_pos > searches_pos,
            "Entities should come after suggested searches"
        );
    }

    #[test]
    fn test_structured_illumination_to_legacy_text_no_searches() {
        let illumination = Illumination {
            meta: IlluminationMeta {
                provider_name: "testilluminator".to_string(),
            },
            summary: "A simple image.".to_string(),
            details: "Nothing remarkable here.".to_string(),
            entities: vec![],
            suggested_searches: vec![],
        };

        let legacy = illumination.to_legacy_text();

        assert!(legacy.contains("A simple image."));
        assert!(legacy.contains("Nothing remarkable here."));
        assert!(!legacy.contains("Entities:"));
        assert!(!legacy.contains("Suggested searches:"));
    }

    #[test]
    fn test_structured_illumination_deserialize() {
        let json = r#"{
            "summary": "Test summary",
            "details": "Test details",
            "entities": [
                {"name": "Entity1", "description": "Description1", "type": "person"},
                {"name": "Entity2", "description": "Description2", "type": "place"}
            ],
            "suggested_searches": ["search1", "search2"]
        }"#;

        let parsed: Illumination = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.summary, "Test summary");
        assert_eq!(parsed.details, "Test details");
        assert_eq!(parsed.entities.len(), 2);
        assert_eq!(parsed.entities[0].name, "Entity1");
        assert_eq!(parsed.entities[0].description, "Description1");
        assert_eq!(parsed.entities[0].entity_type, EntityType::Person);
        assert_eq!(parsed.entities[1].entity_type, EntityType::Place);
        assert_eq!(parsed.suggested_searches.len(), 2);
        assert_eq!(parsed.suggested_searches[0], "search1");
    }

    #[test]
    fn test_structured_illumination_serialize() {
        let illumination = Illumination {
            meta: IlluminationMeta {
                provider_name: "testilluminator".to_string(),
            },
            summary: "Test".to_string(),
            details: "Details".to_string(),
            entities: vec![Entity {
                name: "TestEntity".to_string(),
                description: "TestDesc".to_string(),
                entity_type: EntityType::Brand,
            }],
            suggested_searches: vec!["search".to_string()],
        };

        let json = serde_json::to_string(&illumination).unwrap();

        assert!(json.contains("\"summary\":\"Test\""));
        assert!(json.contains("\"details\":\"Details\""));
        assert!(json.contains("\"entities\":"));
        assert!(json.contains("\"name\":\"TestEntity\""));
        assert!(json.contains("\"description\":\"TestDesc\""));
        assert!(json.contains("\"type\":\"brand\""));
        assert!(json.contains("\"suggested_searches\":[\"search\"]"));
    }
}
