use std::fmt::Write as _;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::illumination::KnowledgeEntityType;

/// Structured v2 interpretation of a capture, with its original JSON retained.
#[derive(Debug, Clone, Serialize)]
pub struct Illumination {
    pub summary: String,
    pub details: String,
    pub suggested_searches: Vec<String>,
    pub entities: Vec<Entity>,

    #[serde(skip)]
    raw_json: Value,
}

#[derive(Deserialize)]
struct IlluminationFields {
    summary: String,
    details: String,
    suggested_searches: Vec<String>,
    entities: Vec<Entity>,
}

impl Illumination {
    pub fn from_raw_json(content: Value) -> Result<Self, serde_json::Error> {
        let fields: IlluminationFields = serde_json::from_value(content.clone())?;

        Ok(Self {
            summary: fields.summary,
            details: fields.details,
            suggested_searches: fields.suggested_searches,
            entities: fields.entities,
            raw_json: content,
        })
    }
}

impl<'de> Deserialize<'de> for Illumination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let content = Value::deserialize(deserializer)?;
        Self::from_raw_json(content).map_err(serde::de::Error::custom)
    }
}

impl Illumination {
    pub fn raw_json(&self) -> &Value {
        &self.raw_json
    }

    /// Lossy Markdown projection for evaluation and console display.
    pub fn to_markdown(&self) -> String {
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
                if let Some(search_query) = &entity.search_query {
                    writeln!(markdown, "  - Search query: {search_query}")
                        .expect("writing to a String cannot fail");
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub entity_type: KnowledgeEntityType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_link: Option<PlatformLink>,
}




/// The platforms the illuminator is allowed to name.
///
/// The list is a prompt vocabulary as much as a type: the prompt is built from `Platform::iter()`, so
/// adding a platform is a variant here and nothing else. Aliases are tolerated on the way in —
/// models have emitted `x` and `twitter` for the same thing — and normalise to `x_twitter` on the way
/// out, which is what the database stores.
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
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Platform {
    /// The explicit strings are also what `EnumString` accepts, in order: the canonical name first
    /// (that is what `Display` and the database use), then the spellings models have emitted.
    #[strum(serialize = "x_twitter")]
    #[strum(serialize = "twitter")]
    #[strum(serialize = "x")]
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
    Github,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformLink {
    pub platform: Platform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// A post-inference URL; never populated from model output.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl PlatformLink {
    /// The database's flattening: three nullable columns, of which the platform is the only one a
    /// link cannot exist without.
    ///
    /// An unrecognised platform becomes [`Platform::Other`] rather than failing: the row is a record
    /// of what was seen, and a platform we have not taught the enum about is not a corrupt row.
    pub fn from_columns(
        platform: Option<String>,
        handle: Option<String>,
        display_name: Option<String>,
    ) -> Option<Self> {
        let platform = Platform::from_str(&platform?).unwrap_or(Platform::Other);
        Some(Self {
            platform,
            handle,
            display_name,
            url: None,
        })
    }

    /// The canonical platform string, which is what the database column holds.
    pub fn platform_name(&self) -> &str {
        self.platform.as_ref()
    }

    /// The handle, when there is one. A link with a display name and no handle is still a link.
    pub fn handle(&self) -> Option<&str> {
        self.handle.as_deref()
    }

    /// The key identity turns on: `(platform, handle)`, folded the same way everywhere.
    pub fn key(&self) -> Option<String> {
        let handle = self.handle()?;
        Some(format!("{}:{}", self.platform_name(), handle.trim().to_lowercase()))
    }
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
                    "search_query": "NFC West Meme War subreddit",
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
                }
            ],
            "future_schema_field": {"preserved": true}
        });

        let illumination =
            Illumination::from_raw_json(content.clone()).expect("v2 output should deserialize");

        assert_eq!(
            illumination.entities[0].entity_type,
            KnowledgeEntityType::OnlineCommunity
        );
        assert_eq!(
            illumination.entities[1].entity_type,
            KnowledgeEntityType::SocialMediaAccount
        );
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
        // `url` is `skip_deserializing`: a model-supplied URL must not survive parsing.
        assert!(reddit_link.url.is_none());
        assert_eq!(illumination.entities[1].description, None);
        assert_eq!(
            illumination.entities[0].search_query.as_deref(),
            Some("NFC West Meme War subreddit")
        );
        assert_eq!(illumination.entities[1].search_query, None);
        assert_eq!(
            illumination.raw_json()["future_schema_field"]["preserved"],
            json!(true)
        );
    }

    #[test]
    fn to_markdown_includes_platform_link_and_optional_entity_fields() {
        let illumination = Illumination::from_raw_json(json!({
            "summary": "A subreddit for NFC West memes",
            "details": "A community about the NFL's NFC West division.",
            "suggested_searches": ["NFC West memes"],
            "entities": [
                {
                    "name": "NFCWestMemeWar",
                    "search_query": "NFC West Meme War subreddit",
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
                }
            ]
        }))
        .unwrap();

        let markdown = illumination.to_markdown();

        assert!(markdown.contains("## Summary\n\nA subreddit for NFC West memes"));
        assert!(markdown.contains("## Details\n\nA community about the NFL's NFC West division."));
        assert!(markdown.contains("- **NFCWestMemeWar** (`online_community`)"));
        assert!(markdown.contains("A community for NFC West memes."));
        assert!(markdown.contains("  - Search query: NFC West Meme War subreddit"));
        assert!(markdown.contains("Platform: reddit"));
        assert!(markdown.contains("display name: NFC West Meme War"));
        assert!(markdown.contains("handle: r/NFCWestMemeWar"));
        assert!(markdown.contains("- **Anonymous account** (`social_media_account`)"));
        assert!(markdown.contains("- NFC West memes"));
    }
}
