use anyhow::Context;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::{
    api,
    llms::{self, InferenceMetadata},
    storage,
};

use super::*;

const PROMPT_VERSION: &str = "capture_illumination_v2";

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
        model_id,
        "Starting v2 illumination inference"
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
        .with_context(|| format!("Failed to parse v2 illumination for capture {}", capture.id))
}

fn make_schema() -> serde_json::Value {
    let entity_types: Vec<String> = EntityType::iter()
        .map(|entity_type| entity_type.as_ref().to_string())
        .collect();
    let platforms: Vec<String> = Platform::iter()
        .map(|platform| platform.as_ref().to_string())
        .collect();

    json!({
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "A concise summary of the image content, no more than 280 characters."
            },
            "details": {
                "type": "string",
                "description": "A detailed description exploring the image content, context, and significance."
            },
            "suggested_searches": {
                "type": "array",
                "description": "Concise searches for notable content worth exploring further.",
                "items": { "type": "string" }
            },
            "entities": {
                "type": "array",
                "description": "Notable entities using one unified entity shape.",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "description": {
                            "type": "string",
                            "description": "Useful information about the entity itself; omit when unsupported or not useful."
                        },
                        "type": {
                            "type": "string",
                            "description": "The entity's category.",
                            "enum": entity_types
                        },
                        "platform_link": {
                            "type": "object",
                            "description": "Optional platform profile clearly associated with this entity.",
                            "properties": {
                                "platform": {
                                    "type": "string",
                                    "enum": platforms
                                },
                                "handle": { "type": "string" },
                                "url": { "type": "string" }
                            },
                            "required": ["platform"]
                        }
                    },
                    "required": ["name", "type"]
                }
            }
        },
        "required": ["summary", "details", "suggested_searches", "entities"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_schema_uses_unified_entity_and_platform_link_shapes() {
        let schema = make_schema();
        let entity_schema = &schema["properties"]["entities"]["items"];
        let entity_types = entity_schema["properties"]["type"]["enum"]
            .as_array()
            .expect("entity types should be an enum");
        let platforms =
            entity_schema["properties"]["platform_link"]["properties"]["platform"]["enum"]
                .as_array()
                .expect("platforms should be an enum");

        assert!(entity_types.contains(&json!("online_community")));
        assert!(entity_types.contains(&json!("social_media_account")));
        assert!(platforms.contains(&json!("reddit")));
        assert!(entity_schema["properties"].get("platform_link").is_some());
        assert!(schema["properties"].get("social_media_accounts").is_none());
        assert_eq!(entity_schema["required"], json!(["name", "type"]));
    }
}
