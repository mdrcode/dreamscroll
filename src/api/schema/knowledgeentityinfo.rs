use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlatformLinkInfo {
    pub platform: String,
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeEntityInfo {
    pub id: i32,
    pub capture_id: i32,
    /// The illumination that produced this occurrence. A capture can be
    /// re-illuminated, so this is what distinguishes a superseded occurrence
    /// from a current one.
    pub illumination_id: i32,
    /// Archived captures stay readable; only the feeds hide them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_archived_at: Option<DateTime<Utc>>,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_link: Option<PlatformLinkInfo>,
}

impl KnowledgeEntityInfo {
    /// Build the wire form of one occurrence.
    ///
    /// `capture_archived_at` is a capture fact, so it is passed in rather than
    /// read from the entity model; callers that filter archived captures pass
    /// `None` because they would not return the row otherwise.
    pub fn new(
        entity: model::knowledge_entity::ModelEx,
        capture_archived_at: Option<DateTime<Utc>>,
    ) -> Self {
        let platform_link = match (entity.platform, entity.platform_handle) {
            (Some(platform), Some(handle)) => Some(PlatformLinkInfo {
                platform,
                handle,
                display_name: entity.platform_display_name,
            }),
            _ => None,
        };

        Self {
            id: entity.id,
            capture_id: entity.capture_id,
            illumination_id: entity.illumination_id,
            capture_archived_at,
            name: entity.name,
            entity_type: entity.entity_type,
            description: entity.description,
            search_query: entity.search_query,
            platform_link,
        }
    }
}
