use crate::api;

/// Represents either a KNode or SocialMedia entity with its associated capture
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "entity_type")]
pub enum EntityInfo {
    KNode {
        id: i32,
        name: String,
        description: String,
        k_type: String,
        capture: api::CaptureInfo,
    },
    SocialMedia {
        id: i32,
        display_name: String,
        handle: String,
        platform: String,
        capture: api::CaptureInfo,
    },
}

impl EntityInfo {
    pub fn entity_id(&self) -> i32 {
        match self {
            EntityInfo::KNode { id, .. } => *id,
            EntityInfo::SocialMedia { id, .. } => *id,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            EntityInfo::KNode { name, .. } => name,
            EntityInfo::SocialMedia { display_name, .. } => display_name,
        }
    }

    pub fn entity_type_label(&self) -> &str {
        match self {
            EntityInfo::KNode { k_type, .. } => k_type,
            EntityInfo::SocialMedia { platform, .. } => platform,
        }
    }
}
