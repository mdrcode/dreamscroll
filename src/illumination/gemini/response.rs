use serde::Deserialize;
use serde_json::json;

pub fn make_response_schema(
    knode_types: Vec<String>,
    social_platform_types: Vec<String>,
) -> serde_json::Value {
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
