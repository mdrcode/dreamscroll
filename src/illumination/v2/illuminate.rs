use anyhow::Context;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::{api, illumination::KnowledgeEntityType, llms, storage};

use super::*;

const PROMPT_VERSION: &str = "capture_illumination_v2";

#[tracing::instrument(skip(client, storage_provider, capture), fields(capture_id = %capture.id))]
pub async fn illuminate(
    client: &dyn llms::InferenceClient,
    storage_provider: &dyn storage::StorageProvider,
    capture: &api::CaptureInfo,
    model_id: &str,
    thinking_level: Option<llms::gemini::ThinkingLevel>,
) -> anyhow::Result<llms::InferenceOutcome<Illumination>> {
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
    let mut inference = client
        .infer(llms::InferenceRequest {
            model_id,
            input: &input,
            response_schema: Some(&schema),
            capabilities: &capabilities,
            thinking_level,
        })
        .await?;

    tracing::info!(
        capture.id,
        provider_request_id = ?inference.metadata.provider_metadata.get("provider_request_id"),
        inference_ms = inference.metadata.duration_ms,
        "Illumination inference output received"
    );

    inference
        .metadata
        .params
        .insert("prompt_version".to_string(), json!(PROMPT_VERSION));
    Ok(llms::InferenceOutcome {
        output: inference.output.and_then(|output| {
            Illumination::from_raw_json(output).with_context(|| {
                format!("Failed to parse v2 illumination for capture {}", capture.id)
            })
        }),
        metadata: inference.metadata,
        http_status_code: inference.http_status_code,
    })
}

fn make_schema() -> serde_json::Value {
    let entity_types: Vec<String> = KnowledgeEntityType::iter()
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
                        "search_query": {
                            "type": "string",
                            "description": "Optional concise entity-specific search query for disambiguation or lookup."
                        },
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
                                "display_name": {
                                    "type": "string",
                                    "description": "Optional name visibly shown by the platform; distinct from the entity name and handle."
                                },
                                "handle": { "type": "string" }
                            },
                            "required": ["platform", "handle"]
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
    fn response_schema_matches_the_entity_shape_the_parser_accepts() {
        let schema = make_schema();
        let entity_schema = &schema["properties"]["entities"]["items"];
        let platform_link_schema = &entity_schema["properties"]["platform_link"];

        assert_eq!(entity_schema["required"], json!(["name", "type"]));
        assert_eq!(
            platform_link_schema["required"],
            json!(["platform", "handle"])
        );
        // `url` is `skip_deserializing`, so the prompt must not ask the model for one.
        assert!(platform_link_schema["properties"].get("url").is_none());
        // v1's separate accounts array must not leak into the v2 contract.
        assert!(schema["properties"].get("social_media_accounts").is_none());

        // The enums are built with `AsRefStr` while the parser reads with serde: if the two
        // spellings diverged, the model would be constrained to values we then reject.
        let entity_types = entity_schema["properties"]["type"]["enum"]
            .as_array()
            .expect("entity types should be an enum");
        assert!(entity_types.contains(&json!("online_community")));
        let platforms = platform_link_schema["properties"]["platform"]["enum"]
            .as_array()
            .expect("platforms should be an enum");
        assert!(platforms.contains(&json!("reddit")));
    }
}
