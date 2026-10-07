//! This module provides a well-defined JSON response schema when using
//! the Gemini API's structured output feature for Illumination tasks.
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
//! The structured response for Illumination tasks contains:
//! - `summary`: A concise 1-2 sentence summary (max ~240 chars)
//! - `details`: A more detailed multi-paragraph description
//! - `suggested_searches`: A list of search queries to learn more
//! - `entities`: A list of notable entities with descriptions and types
//!   (person, place, book, movie, television_show, etc). See `EntityType`
//!   enum for full list)
//! - `social_media_accounts`: A list of social media accounts with
//!   display_name, handle, and platform
//!

use serde::Deserialize;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::illumination;

#[derive(Deserialize, Debug)]
pub struct GeminiStructuredResponse {
    pub summary: String,
    pub details: String,
    pub suggested_searches: Vec<String>,
    pub entities: Vec<illumination::Entity>,
    pub social_media_accounts: Vec<illumination::SocialMediaAccount>,
}

impl From<GeminiStructuredResponse> for illumination::Illumination {
    fn from(resp: GeminiStructuredResponse) -> Self {
        illumination::Illumination {
            meta: illumination::IlluminationMeta {
                provider_name: "gemini".to_string(),
            },
            summary: resp.summary,
            details: resp.details,
            suggested_searches: resp.suggested_searches,
            entities: resp.entities,
            social_media_accounts: resp.social_media_accounts,
        }
    }
}

// Build the JSON Schema sent to the Interactions API.
pub fn make_response_schema() -> serde_json::Value {
    let entity_types: Vec<String> = illumination::EntityType::iter()
        .map(|entity_type| entity_type.as_ref().to_string())
        .collect();
    let platform_types: Vec<String> = illumination::SocialMediaPlatform::iter()
        .map(|platform| platform.as_ref().to_string())
        .collect();

    json!({
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "A concise 1-2 sentence summary of the image content, max 240 characters. Focus on substance, not format."
            },
            "details": {
                "type": "string",
                "description": "A detailed multi-paragraph description exploring the content, context, and significance of the image."
            },
            "suggested_searches": {
                "type": "array",
                "description": "A list of concise search queries for notable objects, people, or locations visible in the image.",
                "items": { "type": "string" }
            },
            "entities": {
                "type": "array",
                "description": "A list of notable entities (objects, people, locations, references) with descriptions and types. Do NOT include social media accounts here.",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the entity"
                        },
                        "description": {
                            "type": "string",
                            "description": "A brief description of the entity"
                        },
                        "type": {
                            "type": "string",
                            "description": "The type of entity",
                            "enum": entity_types
                        }
                    },
                    "required": ["name", "description", "type"]
                }
            },
            "social_media_accounts": {
                "type": "array",
                "description": "A list of social media accounts visible in the image.",
                "items": {
                    "type": "object",
                    "properties": {
                        "display_name": {
                            "type": "string",
                            "description": "The display name or real name shown on the profile"
                        },
                        "handle": {
                            "type": "string",
                            "description": "The username/handle of the account (e.g., @username)"
                        },
                        "platform": {
                            "type": "string",
                            "description": "The social-media platform",
                            "enum": platform_types
                        }
                    },
                    "required": ["display_name", "handle", "platform"]
                }
            }
        },
        "required": ["summary", "details", "suggested_searches", "entities", "social_media_accounts"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_schema_uses_json_schema_type_names() {
        let schema = make_response_schema();

        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["properties"]["summary"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["entities"]["items"]["type"],
            json!("object")
        );
        assert!(
            schema["properties"]["entities"]["items"]["properties"]["type"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("real_person"))
        );
    }
}
