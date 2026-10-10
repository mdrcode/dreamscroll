use std::fmt::Write as _;

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

    fn to_markdown(&self) -> String {
        let mut markdown = String::new();
        writeln!(
            markdown,
            "## Summary\n\n{}\n\n## Details\n\n{}",
            self.summary, self.details
        )
        .expect("writing to a String cannot fail");

        if !self.suggested_searches.is_empty() {
            markdown.push_str("\n## Suggested searches\n");
            for search in &self.suggested_searches {
                writeln!(markdown, "- {search}").expect("writing to a String cannot fail");
            }
        }

        if !self.entities.is_empty() {
            markdown.push_str("\n## Entities\n");
            for entity in &self.entities {
                write!(markdown, "- **{}** (`{}`)", entity.name, entity.entity_type)
                    .expect("writing to a String cannot fail");
                if let Some(description) = &entity.description {
                    write!(markdown, " — {description}").expect("writing to a String cannot fail");
                }
                markdown.push('\n');

                if let Some(link) = &entity.platform_link {
                    write!(markdown, "  - Platform: {}", link.platform)
                        .expect("writing to a String cannot fail");
                    if let Some(display_name) = &link.display_name {
                        write!(markdown, "; display name: {display_name}")
                            .expect("writing to a String cannot fail");
                    }
                    if let Some(handle) = &link.handle {
                        write!(markdown, "; handle: {handle}")
                            .expect("writing to a String cannot fail");
                    }
                    if let Some(url) = &link.url {
                        write!(markdown, "; URL: {url}").expect("writing to a String cannot fail");
                    }
                    markdown.push('\n');
                }
            }
        }

        markdown
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
    Event,
    Book,
    Movie,
    TelevisionShow,
    ArtWork,
    FictionalCharacter,
    Music,
    Meme,
    Software,
    Product,
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
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// Optional post-inference URL; not populated from model output.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
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
                        "display_name": "NFC West Meme War",
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
                },
                {
                    "name": "Venice International Film Festival",
                    "description": "An annual film festival for international cinema.",
                    "type": "event"
                },
                {
                    "name": "Nikon Z8",
                    "description": "A camera model.",
                    "type": "product"
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
        assert_eq!(illumination.entities[2].entity_type, EntityType::Event);
        assert_eq!(illumination.entities[3].entity_type, EntityType::Product);
        let reddit_link = illumination.entities[0]
            .platform_link
            .as_ref()
            .expect("community link should be present");
        assert_eq!(reddit_link.platform, Platform::Reddit);
        assert_eq!(reddit_link.handle.as_deref(), Some("r/NFCWestMemeWar"));
        assert_eq!(
            reddit_link.display_name.as_deref(),
            Some("NFC West Meme War")
        );
        assert!(reddit_link.url.is_none());
        assert_eq!(illumination.entities[1].description, None);
        assert_eq!(
            illumination.raw_json()["future_schema_field"]["preserved"],
            json!(true)
        );
    }

    #[test]
    fn to_markdown_includes_platform_link_and_optional_entity_fields() {
        let illumination = Illumination::from_raw_json(
            json!({
                "summary": "A subreddit for NFC West memes",
                "details": "A community about the NFL's NFC West division.",
                "suggested_searches": ["NFC West memes"],
                "entities": [
                    {
                        "name": "NFCWestMemeWar",
                        "description": "A community for NFC West memes.",
                        "type": "online_community",
                        "platform_link": {
                            "platform": "reddit",
                            "display_name": "NFC West Meme War",
                            "handle": "r/NFCWestMemeWar"
                        }
                    },
                    {
                        "name": "Anonymous account",
                        "type": "social_media_account"
                    },
                    {
                        "name": "Venice International Film Festival",
                        "description": "An annual film festival for international cinema.",
                        "type": "event"
                    },
                    {
                        "name": "Nikon Z8",
                        "description": "A camera model.",
                        "type": "product"
                    }
                ]
            }),
            InferenceMetadata::default(),
        )
        .unwrap();

        let markdown = illumination.to_markdown();

        assert!(markdown.contains("## Summary\n\nA subreddit for NFC West memes"));
        assert!(markdown.contains("## Details\n\nA community about the NFL's NFC West division."));
        assert!(markdown.contains(
            "- **NFCWestMemeWar** (`online_community`) — A community for NFC West memes."
        ));
        assert!(markdown.contains(
            "Platform: reddit; display name: NFC West Meme War; handle: r/NFCWestMemeWar"
        ));
        assert!(markdown.contains(
            "- **Venice International Film Festival** (`event`) — An annual film festival for international cinema."
        ));
        assert!(markdown.contains("- **Nikon Z8** (`product`) — A camera model."));
        assert!(markdown.contains("- **Anonymous account** (`social_media_account`)"));
        assert!(markdown.contains("- NFC West memes"));
    }
}
