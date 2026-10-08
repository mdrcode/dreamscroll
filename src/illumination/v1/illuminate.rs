use anyhow::Context;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::{
    api,
    llms::{self, InferenceMetadata},
    storage,
};

use super::*;

const PROMPT_VERSION: &str = "capture_illumination_v1";

#[tracing::instrument(skip(client, storage_provider, capture), fields(capture_id = %capture.id))]
pub async fn illuminate(
    client: &dyn llms::InferenceClient,
    storage_provider: &dyn storage::StorageProvider,
    capture: &api::CaptureInfo,
    model_id: &str,
    inference_id: String,
) -> anyhow::Result<Illumination> {
    let media = capture
        .medias
        .first()
        .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;
    let storage_handle = storage::StorageHandle::from(media);
    let image = storage_provider.retrieve_bytes(&storage_handle).await?;
    let mime_type = media.mime_type.as_deref().unwrap_or("image/jpeg");
    let input = [
        llms::InferenceInput::Text(prompt::PROMPT),
        llms::InferenceInput::Image {
            bytes: image.as_ref(),
            mime_type,
        },
    ];
    let schema = make_schema();
    let capabilities = [llms::InferenceCapability::WebSearch];

    tracing::info!(
        capture.id,
        media.id,
        image_bytes = image.len(),
        mime_type,
        "Starting v1 illumination inference"
    );
    let inference_start = std::time::Instant::now();
    let inference = client
        .infer(llms::InferenceRequest {
            model_id,
            input: &input,
            response_schema: Some(&schema),
            capabilities: &capabilities,
        })
        .await?;
    let duration_ms = inference_start.elapsed().as_millis() as i64;

    tracing::info!(
        capture.id,
        provider_request_id = ?inference.provider_request_id,
        inference_ms = duration_ms,
        "Illumination inference output received"
    );

    let inference_metadata = InferenceMetadata {
        inference_id,
        prompt_version: PROMPT_VERSION.to_string(),
        provider_name: client.provider_name().to_string(),
        backend_name: client.backend_name().to_string(),
        model_id: model_id.to_string(),
        duration_ms,
        provider_request_id: inference.provider_request_id,
        provider_usage_json: inference.provider_usage_json,
    };
    Illumination::from_raw_json(inference.output, inference_metadata)
        .with_context(|| format!("Failed to parse v1 illumination for capture {}", capture.id))
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
