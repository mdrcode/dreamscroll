use anyhow::Context;
use serde_json::json;
use strum::IntoEnumIterator;

use crate::{api, illumination::KnowledgeEntityType, llms, storage};

use super::annotation_hint::annotation_hint;
use super::*;

#[tracing::instrument(skip(client, storage_provider, capture), fields(capture_id = %capture.id))]
pub async fn illuminate(
    client: &dyn llms::InferenceClient,
    storage_provider: &dyn storage::StorageProvider,
    capture: &api::CaptureInfo,
    model_id: &str,
    thinking_level: Option<llms::gemini::ThinkingLevel>,
    capabilities: &[llms::InferenceCapability],
) -> anyhow::Result<llms::InferenceOutcome<Illumination>> {
    let media = capture
        .medias
        .first()
        .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;
    let storage_handle = storage::StorageHandle::from(media);
    let image = storage_provider.retrieve_bytes(&storage_handle).await?;
    let mime_type = media.mime_type.as_deref().unwrap_or("image/jpeg");
    // The note trails the image: the base prompt and image stay a stable prefix, and the note is
    // the last thing the model reads.
    let note = capture
        .annotation
        .as_ref()
        .and_then(|annotation| annotation_hint(&annotation.content));
    let input = input_parts(image.as_ref(), mime_type, note.as_deref());
    let schema = make_schema();
    tracing::info!(
        capture.id,
        media.id,
        image_bytes = image.len(),
        mime_type,
        model_id,
        has_annotation = note.is_some(),
        "Starting v2 illumination inference"
    );
    let mut inference = client
        .infer(llms::InferenceRequest {
            model_id,
            input: &input,
            response_schema: Some(&schema),
            capabilities,
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
        .insert("prompt_version".to_string(), json!(prompt::PROMPT_VERSION));
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

fn input_parts<'a>(
    image: &'a [u8],
    mime_type: &'a str,
    note: Option<&'a str>,
) -> Vec<llms::InferenceInput<'a>> {
    let mut input = vec![
        llms::InferenceInput::Text(prompt::PROMPT),
        llms::InferenceInput::Image {
            bytes: image,
            mime_type,
        },
    ];
    if let Some(note) = note {
        input.push(llms::InferenceInput::Text(note));
    }
    input
}

fn make_schema() -> serde_json::Value {
    let entity_types: Vec<String> = KnowledgeEntityType::iter()
        .map(|entity_type| entity_type.as_ref().to_string())
        .collect();
    let known_platforms = Platform::KNOWN.join(", ");

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
                                    "description": format!(
                                        "Canonical platform name. Prefer one of: {known_platforms}. \
                                         If none fits, use the platform name shown."
                                    ),
                                    "examples": Platform::KNOWN
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
    fn owner_note_trails_the_prompt_and_image() {
        let image = [0u8; 4];
        let note = annotation_hint("The sign reads Nias.").unwrap();

        let base = input_parts(&image, "image/png", None);
        assert_eq!(base.len(), 2);
        assert!(matches!(base[0], llms::InferenceInput::Text(text) if text == prompt::PROMPT));
        assert!(
            matches!(base[1], llms::InferenceInput::Image { mime_type, .. } if mime_type == "image/png")
        );

        let with_note = input_parts(&image, "image/png", Some(&note));
        assert_eq!(with_note.len(), 3);
        assert!(
            matches!(with_note[2], llms::InferenceInput::Text(text) if text.contains("The sign reads Nias."))
        );
    }

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

        // Entity types stay constrained; platforms are open so a new platform survives instead of
        // being coerced to `other`.
        let entity_types = entity_schema["properties"]["type"]["enum"]
            .as_array()
            .expect("entity types should be an enum");
        assert!(entity_types.contains(&json!("online_community")));
        let platform = &platform_link_schema["properties"]["platform"];
        assert_eq!(platform["type"], "string");
        assert!(platform.get("enum").is_none());
        assert!(
            platform["examples"]
                .as_array()
                .expect("known platforms should be examples")
                .contains(&json!("reddit"))
        );
        assert!(
            platform["description"]
                .as_str()
                .expect("the platform schema describes the vocabulary")
                .contains("x_twitter")
        );
    }
}
