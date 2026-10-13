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

impl From<model::knowledge_entity::ModelEx> for KnowledgeEntityInfo {
    fn from(entity: model::knowledge_entity::ModelEx) -> Self {
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
            name: entity.name,
            entity_type: entity.entity_type,
            description: entity.description,
            search_query: entity.search_query,
            platform_link,
        }
    }
}
