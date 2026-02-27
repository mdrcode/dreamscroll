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
                provider_name: "geministructured".to_string(),
            },
            summary: resp.summary,
            details: resp.details,
            suggested_searches: resp.suggested_searches,
            entities: resp.entities,
            social_media_accounts: resp.social_media_accounts,
        }
    }
}

// Build the JSON schema for the structured response
// Following the OpenAPI 3.0 schema format that Gemini expects
pub fn make_response_schema() -> serde_json::Value {
    let knode_types: Vec<String> = illumination::EntityType::iter()
        .map(|e| e.as_ref().to_string())
        .collect();
    let social_platform_types: Vec<String> = illumination::SocialMediaPlatform::iter()
        .map(|e| e.as_ref().to_string())
        .collect();

    json!({
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
                "description": "A list of notable entities (objects, people, locations, references) with descriptions and types. Do NOT include social media accounts here.",
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
                            "description": "The type of entity: real_person, place, book, movie, television_show, art_work, fictional_character, music, meme, software, financial, brand, or unknown",
                            "enum": knode_types
                        }
                    },
                    "required": ["name", "description", "type"]
                }
            },
            "social_media_accounts": {
                "type": "ARRAY",
                "description": "A list of social media accounts visible in the image.",
                "items": {
                    "type": "OBJECT",
                    "properties": {
                        "display_name": {
                            "type": "STRING",
                            "description": "The display name or real name shown on the profile"
                        },
                        "handle": {
                            "type": "STRING",
                            "description": "The username/handle of the account (e.g., @username)"
                        },
                        "platform": {
                            "type": "STRING",
                            "description": "The platform: x_twitter, youtube, instagram, tiktok, facebook, linkedin, threads, bluesky, mastodon, other",
                            "enum": social_platform_types
                        }
                    },
                    "required": ["display_name", "handle", "platform"]
                }
            }
        },
        "required": ["summary", "details", "suggested_searches", "entities", "social_media_accounts"]
    })
}

#[derive(Deserialize, Debug)]
pub struct GeminiRawContent {
    pub candidates: Vec<RawContentCandidate>,
}

#[derive(Deserialize, Debug)]
pub struct RawContentCandidate {
    pub content: RawContentParts,
}

#[derive(Deserialize, Debug)]
pub struct RawContentParts {
    pub parts: Vec<RawPart>,
}

#[derive(Deserialize, Debug)]
pub struct RawPart {
    pub text: String,
}
