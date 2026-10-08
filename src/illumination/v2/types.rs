use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::llms::{InferenceMetadata, InferenceResult};

/// Structured v2 interpretation of a capture, with its original JSON retained.
#[derive(Debug, Clone, Serialize)]
pub struct Illumination {
    pub summary: String,
    pub details: String,
    pub suggested_searches: Vec<String>,
    pub entities: Vec<Entity>,

    #[serde(skip)]
    raw_json: Value,
    #[serde(skip)]
    inference_metadata: InferenceMetadata,
}

#[derive(Deserialize)]
struct IlluminationFields {
    summary: String,
    details: String,
    suggested_searches: Vec<String>,
    entities: Vec<Entity>,
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
    Organization,
    OnlineCommunity,
    SocialMediaAccount,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_link: Option<PlatformLink>,
}

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
pub enum Platform {
    XTwitter,
    Youtube,
    Instagram,
    Tiktok,
    Facebook,
    Linkedin,
    Threads,
    Bluesky,
    Mastodon,
    Reddit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformLink {
    pub platform: Platform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn illumination_deserializes_unified_v2_entities() {
        let content = json!({
            "summary": "A Reddit community for NFC West memes",
            "details": "A community focused on memes about the NFL's NFC West division.",
            "suggested_searches": [],
            "entities": [
                {
                    "name": "NFCWestMemeWar",
                    "description": "A community for memes about the NFL's NFC West division.",
                    "type": "online_community",
                    "platform_link": {
                        "platform": "reddit",
                        "handle": "r/NFCWestMemeWar",
                        "url": "https://www.reddit.com/r/NFCWestMemeWar/"
                    }
                },
                {
                    "name": "@anonhandle",
                    "type": "social_media_account",
                    "platform_link": {
                        "platform": "x_twitter",
                        "handle": "@anonhandle"
                    }
                }
            ],
            "future_schema_field": {"preserved": true}
        });

        let illumination =
            Illumination::from_raw_json(content.clone(), InferenceMetadata::default())
                .expect("v2 output should deserialize");

        assert_eq!(
            illumination.entities[0].entity_type,
            EntityType::OnlineCommunity
        );
        let reddit_link = illumination.entities[0]
            .platform_link
            .as_ref()
            .expect("community link should be present");
        assert_eq!(reddit_link.platform, Platform::Reddit);
        assert_eq!(reddit_link.handle.as_deref(), Some("r/NFCWestMemeWar"));
        assert_eq!(illumination.entities[1].description, None);
        assert_eq!(
            illumination.raw_json()["future_schema_field"]["preserved"],
            json!(true)
        );
    }
}
