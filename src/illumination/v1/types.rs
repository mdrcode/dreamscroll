use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::llms::{InferenceMetadata, InferenceResult};

/// Structured v1 interpretation of a capture, with the original result retained for persistence.
#[derive(Debug, Clone, Serialize)]
pub struct Illumination {
    /// A concise 1-2 sentence summary of the capture content (max 280 chars).
    /// Suitable for display in a list view alongside other summaries.
    pub summary: String,

    /// A detailed multi-paragraph description of the capture content.
    /// Explores the content, context, and significance of the capture.
    pub details: String,

    /// A list of suggested search queries to learn more about the capture content.
    /// Each entry is a concise search query for a notable object, person, or
    /// location visible in the capture.
    pub suggested_searches: Vec<String>,

    /// A list of notable entities (objects, people, locations, references, etc)
    /// in the capture. Each entry contains the entity name, type, and a description.
    pub entities: Vec<Entity>,

    /// A list of social media accounts visible in the capture.
    /// Each entry contains the display name, handle, and platform.
    pub social_media_accounts: Vec<SocialMediaAccount>,

    #[serde(skip)]
    raw_json: Value,
    #[serde(skip)]
    inference_metadata: InferenceMetadata,
}

// Parses known v1 fields while Illumination retains the full raw JSON.
#[derive(Deserialize)]
struct IlluminationFields {
    summary: String,
    details: String,
    suggested_searches: Vec<String>,
    entities: Vec<Entity>,
    social_media_accounts: Vec<SocialMediaAccount>,
}

impl Illumination {
    pub fn from_raw_json(
        content: Value,
        inference_metadata: InferenceMetadata,
    ) -> Result<Self, serde_json::Error> {
        let fields: IlluminationFields = serde_json::from_value(content.clone())?;

        Ok(Self {
            summary: fields.summary,
            details: fields.details,
            suggested_searches: fields.suggested_searches,
            entities: fields.entities,
            social_media_accounts: fields.social_media_accounts,
            raw_json: content,
            inference_metadata,
        })
    }
}

impl<'de> Deserialize<'de> for Illumination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let content = Value::deserialize(deserializer)?;
        Self::from_raw_json(content, InferenceMetadata::default()).map_err(serde::de::Error::custom)
    }
}

impl InferenceResult for Illumination {
    fn raw_json(&self) -> &Value {
        &self.raw_json
    }

    fn metadata(&self) -> &InferenceMetadata {
        &self.inference_metadata
    }
}

/// The type/category of an entity.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EntityType {
    RealPerson,
    Place,
    Book,
    Movie,
    TelevisionShow,
    ArtWork,
    FictionalCharacter,
    Music,
    Meme,
    Software,
    Financial,
    Brand,
    Unknown,
}

/// Represents a notable entity found in an image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// The name of the entity (object, person, location, or reference).
    pub name: String,

    /// A brief description of the entity.
    pub description: String,

    /// The type/category of the entity.
    #[serde(rename = "type")]
    pub entity_type: EntityType,
}

/// The platform of a social media account.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SocialMediaPlatform {
    XTwitter,
    Youtube,
    Instagram,
    Tiktok,
    Facebook,
    Linkedin, // keep second 'i' small for serialization consistency
    Threads,
    Bluesky,
    Mastodon,
    Other,
}

/// Represents a social media account found in an image.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialMediaAccount {
    /// The display name shown on the account profile.
    pub display_name: String,

    /// The handle/username of the account (e.g., @username).
    pub handle: String,

    /// The platform where this account exists.
    #[serde(rename = "platform")]
    pub platform: SocialMediaPlatform,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illumination_deserializes_v1_output_shape() {
        let illumination: Illumination = serde_json::from_value(serde_json::json!({
            "summary": "A profile for Ada Lovelace",
            "details": "A mathematician and writer.",
            "suggested_searches": [],
            "entities": [{
                "name": "Ada Lovelace",
                "description": "A mathematician and writer.",
                "type": "real_person"
            }],
            "social_media_accounts": [{
                "display_name": "Ada",
                "handle": "@ada",
                "platform": "x_twitter"
            }],
            "future_schema_field": {"nested": {"preserved": true}}
        }))
        .unwrap();

        assert_eq!(illumination.entities[0].entity_type, EntityType::RealPerson);
        assert_eq!(
            illumination.social_media_accounts[0].platform,
            SocialMediaPlatform::XTwitter
        );
        assert_eq!(illumination.social_media_accounts[0].handle, "@ada");
        assert_eq!(
            illumination.raw_json()["future_schema_field"]["nested"]["preserved"],
            serde_json::json!(true)
        );
    }
}
