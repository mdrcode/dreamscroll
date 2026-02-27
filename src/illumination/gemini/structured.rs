use base64::Engine;
use reqwest::Client;
use serde_json::json;

use crate::{api, illumination, storage};

use super::*;

/// Gemini-based illuminator that leverages the public Gemini API to return a GeminiStructuredResponse.
#[derive(Clone)]
pub struct GeminiStructuredIlluminator {
    gemini_api_key: String,
    storage: Box<dyn storage::StorageProvider>,
}

impl GeminiStructuredIlluminator {
    pub fn new(gemini_api_key: String, storage: Box<dyn storage::StorageProvider>) -> Self {
        GeminiStructuredIlluminator {
            gemini_api_key,
            storage,
        }
    }
}

#[async_trait::async_trait]
impl illumination::Illuminator for GeminiStructuredIlluminator {
    fn name(&self) -> &'static str {
        "geministructured"
    }

    /// Illuminates a capture and returns the structured response directly.
    #[tracing::instrument(skip(self, capture), fields(capture_id = %capture.id))]
    async fn illuminate(
        &self,
        capture: &api::CaptureInfo,
    ) -> anyhow::Result<illumination::Illumination> {
        let media1 = capture
            .medias
            .get(0)
            .ok_or_else(|| anyhow::anyhow!("Capture has no media"))?;

        let storage_handle = storage::StorageHandle::from(media1);
        let buffer = self.storage.retrieve_bytes(&storage_handle).await?;

        let enc = base64::engine::general_purpose::STANDARD.encode(buffer);

        tracing::info!(
            "GeminiStructuredIlluminator: capture {} media {} base64 bytes {}",
            capture.id,
            media1.id,
            enc.len()
        );

        let client = Client::new();

        // Prepare request with text + image and structured output config
        let request_body = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {
                        "text": prompts::PROMPT
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
                "responseSchema": response::make_response_schema()
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
            .header("x-goog-api-key", &self.gemini_api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if response.status().is_success() {
            let parse: response::GeminiRawContent = response.json().await?;
            let json = &parse.candidates[0].content.parts[0].text;
            let structured: response::GeminiStructuredResponse = serde_json::from_str(json)
                .map_err(|e| anyhow::anyhow!("Failed to parse structured response: {}", e))?;

            tracing::info!(
                "GeminiStructuredIlluminator: Successfully parsed response for capture {} with {} entities, {} social media accounts, and {} suggested searches",
                capture.id,
                structured.entities.len(),
                structured.social_media_accounts.len(),
                structured.suggested_searches.len()
            );

            Ok(illumination::Illumination::from(structured))
        } else {
            let status_code = response.status();
            let error_text = response.text().await?;
            tracing::error!(
                "GeminiStructuredIlluminator: API failed for capture {} with status {}, error text: {}",
                capture.id,
                status_code,
                error_text
            );
            Err(anyhow::anyhow!(
                "Gemini API Error status {}: {}",
                status_code,
                error_text
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_structured_illumination_to_legacy_text() {
        let illumination = illumination::Illumination {
            meta: illumination::IlluminationMeta { provider_name: "testilluminator".to_string() },
            summary: "A fascinating book about quantum physics.".to_string(),
            details: "This appears to be a cover of a popular science book.\n\nThe author is well-known for making complex topics accessible.".to_string(),
            entities: vec![
                illumination::Entity {
                    name: "Quantum Physics Book".to_string(),
                    description: "A popular science book on quantum mechanics".to_string(),
                    entity_type: illumination::EntityType::Book,
                },
            ],
            suggested_searches: vec![
                "quantum physics introduction".to_string(),
                "author name books".to_string(),
            ],
            social_media_accounts: vec![
                illumination::SocialMediaAccount {
                    display_name: "Science Author".to_string(),
                    handle: "@scienceauthor".to_string(),
                    platform: illumination::SocialMediaPlatform::XTwitter,
                },
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
        assert!(legacy.contains("Social Media Accounts:"));
        assert!(legacy.contains("- Science Author (@scienceauthor) [x_twitter]"));

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
        let illumination = illumination::Illumination {
            meta: illumination::IlluminationMeta {
                provider_name: "testilluminator".to_string(),
            },
            summary: "A simple image.".to_string(),
            details: "Nothing remarkable here.".to_string(),
            entities: vec![],
            suggested_searches: vec![],
            social_media_accounts: vec![],
        };

        let legacy = illumination.to_legacy_text();

        assert!(legacy.contains("A simple image."));
        assert!(legacy.contains("Nothing remarkable here."));
        assert!(!legacy.contains("Entities:"));
        assert!(!legacy.contains("Suggested searches:"));
        assert!(!legacy.contains("Social Media Accounts:"));
    }

    #[test]
    fn test_structured_illumination_deserialize() {
        let json = r#"{
            "summary": "Test summary",
            "details": "Test details",
            "entities": [
                {"name": "Entity1", "description": "Description1", "type": "real_person"},
                {"name": "Entity2", "description": "Description2", "type": "place"}
            ],
            "suggested_searches": ["search1", "search2"],
            "social_media_accounts": [
                {"display_name": "Test User", "handle": "@testuser", "platform": "x_twitter"}
            ]
        }"#;

        // Deserialize to GeminiStructuredResponse (as the API does)
        let structured: response::GeminiStructuredResponse = serde_json::from_str(json).unwrap();
        let parsed = illumination::Illumination::from(structured);

        assert_eq!(parsed.summary, "Test summary");
        assert_eq!(parsed.details, "Test details");
        assert_eq!(parsed.entities.len(), 2);
        assert_eq!(parsed.entities[0].name, "Entity1");
        assert_eq!(parsed.entities[0].description, "Description1");
        assert_eq!(
            parsed.entities[0].entity_type,
            illumination::EntityType::RealPerson
        );
        assert_eq!(
            parsed.entities[1].entity_type,
            illumination::EntityType::Place
        );
        assert_eq!(parsed.suggested_searches.len(), 2);
        assert_eq!(parsed.suggested_searches[0], "search1");
        assert_eq!(parsed.social_media_accounts.len(), 1);
        assert_eq!(parsed.social_media_accounts[0].display_name, "Test User");
        assert_eq!(parsed.social_media_accounts[0].handle, "@testuser");
        assert_eq!(
            parsed.social_media_accounts[0].platform,
            illumination::SocialMediaPlatform::XTwitter
        );
    }

    #[test]
    fn test_structured_illumination_serialize() {
        let illumination = illumination::Illumination {
            meta: illumination::IlluminationMeta {
                provider_name: "testilluminator".to_string(),
            },
            summary: "Test".to_string(),
            details: "Details".to_string(),
            entities: vec![illumination::Entity {
                name: "TestEntity".to_string(),
                description: "TestDesc".to_string(),
                entity_type: illumination::EntityType::Brand,
            }],
            suggested_searches: vec!["search".to_string()],
            social_media_accounts: vec![illumination::SocialMediaAccount {
                display_name: "Test User".to_string(),
                handle: "@testuser".to_string(),
                platform: illumination::SocialMediaPlatform::Youtube,
            }],
        };

        let json = serde_json::to_string(&illumination).unwrap();

        assert!(json.contains("\"summary\":\"Test\""));
        assert!(json.contains("\"details\":\"Details\""));
        assert!(json.contains("\"entities\":"));
        assert!(json.contains("\"name\":\"TestEntity\""));
        assert!(json.contains("\"description\":\"TestDesc\""));
        assert!(json.contains("\"type\":\"brand\""));
        assert!(json.contains("\"suggested_searches\":[\"search\"]"));
        assert!(json.contains("\"social_media_accounts\":"));
        assert!(json.contains("\"display_name\":\"Test User\""));
        assert!(json.contains("\"handle\":\"@testuser\""));
        assert!(json.contains("\"platform\":\"youtube\""));
    }
}
