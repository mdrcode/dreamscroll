use serde::{Deserialize, Serialize};

/// Structured result produced by the v1 capture-analysis prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Illumination {
    /// A concise 1-2 sentence summary of the capture content (max ~240 chars).
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
            }]
        }))
        .unwrap();

        assert_eq!(illumination.entities[0].entity_type, EntityType::RealPerson);
        assert_eq!(
            illumination.social_media_accounts[0].platform,
            SocialMediaPlatform::XTwitter
        );
        assert_eq!(illumination.social_media_accounts[0].handle, "@ada");
    }
}
