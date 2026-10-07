use anyhow::Context;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::{api, llms, storage};

use super::*;

#[tracing::instrument(skip(client, storage_provider, capture), fields(capture_id = %capture.id))]
pub async fn illuminate(
    client: &llms::gemini::GeminiInferenceClient,
    storage_provider: &dyn storage::StorageProvider,
    capture: &api::CaptureInfo,
) -> anyhow::Result<Illumination> {
    let media = capture
        .medias
        .first()
        .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;
    let storage_handle = storage::StorageHandle::from(media);
    let image = storage_provider.retrieve_bytes(&storage_handle).await?;
    let mime_type = media.mime_type.as_deref().unwrap_or("image/jpeg");
    let input = [
        llms::gemini::GeminiInputPart::Text(prompt::PROMPT),
        llms::gemini::GeminiInputPart::InlineImage {
            bytes: image.as_ref(),
            mime_type,
        },
    ];
    let schema = make_schema();
    let tools = [json!({ "type": "google_search" })];

    tracing::info!(
        capture.id,
        media.id,
        image_bytes = image.len(),
        mime_type,
        "Starting Gemini Interactions illumination"
    );
    let inference_start = std::time::Instant::now();
    let interaction = client
        .interact(llms::gemini::GeminiInteractionRequest {
            input: &input,
            response_schema: Some(&schema),
            tools: &tools,
            // Do not retain screenshot interactions server-side; Dreamscroll persists the
            // resulting illumination separately.
            store: false,
        })
        .await?;
    let structured_json = interaction.output_text()?;
    let illumination: Illumination = serde_json::from_str(&structured_json).with_context(|| {
        format!(
            "Failed to parse Gemini illumination JSON for capture {}",
            capture.id
        )
    })?;

    tracing::info!(
        capture.id,
        interaction_id = ?interaction.id,
        num_entities = illumination.entities.len(),
        num_social_media_accounts = illumination.social_media_accounts.len(),
        num_suggested_searches = illumination.suggested_searches.len(),
        gemini_interactions_ms = inference_start.elapsed().as_millis(),
        "Gemini Interactions illumination succeeded"
    );

    Ok(illumination)
}

fn make_schema() -> serde_json::Value {
    let entity_types: Vec<String> = EntityType::iter()
        .map(|entity_type| entity_type.as_ref().to_string())
        .collect();
    let platform_types: Vec<String> = SocialMediaPlatform::iter()
        .map(|platform| platform.as_ref().to_string())
        .collect();

    json!({
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "A concise 1-2 sentence summary of the image content, max 280 characters. Focus on substance, not format."
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
        let schema = make_schema();

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
