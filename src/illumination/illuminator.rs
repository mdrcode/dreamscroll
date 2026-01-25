use serde::{Deserialize, Serialize};

use crate::api;

#[async_trait::async_trait]
pub trait Illuminator: dyn_clone::DynClone + Send + Sync {
    fn model_name(&self) -> &'static str;
    async fn illuminate(&self, capture: api::CaptureInfo) -> anyhow::Result<Illumination>;
}

dyn_clone::clone_trait_object!(Illuminator);

/// Structured response which describes and exposits the content of an image capture.
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

    /// A list of notable entities (objects, people, locations, references) in the capture.
    /// Each entry contains the entity name and a brief description.
    pub entities: Vec<Entity>,
}

/// The type/category of an entity.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumIter,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EntityType {
    Person,
    Place,
    Book,
    Movie,
    TelevisionShow,
    Music,
    ArtWork,
    Meme,
    Software,
    Financial,
    Youtuber,
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

impl Illumination {
    /// Converts the structured illumination back to a legacy freeform text format.
    ///
    /// This is useful for backwards compatibility with the current Illumination
    /// database model which stores a single text blob.
    ///
    /// Format:
    /// ```text
    /// <summary>
    ///
    /// <details>
    ///
    /// Suggested searches:
    /// - <search 1>
    /// - <search 2>
    /// ...
    ///
    /// Entities:
    /// - <entity name> [<type>]: <description>
    /// - <entity name> [<type>]: <description>
    /// ...
    /// ```
    pub fn to_legacy_text(&self) -> String {
        let mut result = format!("{}\n\n{}", self.summary, self.details);

        if !self.suggested_searches.is_empty() {
            result.push_str("\n\nSuggested searches:");
            for search in &self.suggested_searches {
                result.push_str(&format!("\n- {}", search));
            }
        }

        if !self.entities.is_empty() {
            result.push_str("\n\nEntities:");
            for entity in &self.entities {
                result.push_str(&format!(
                    "\n- {} [{}]: {}",
                    entity.name, entity.entity_type, entity.description
                ));
            }
        }

        result
    }
}
