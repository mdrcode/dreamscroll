//! The knowledge entity behind an observation: the referent, not the reading.
//!
//! An observation is personal; the entity it attaches to is shared. This is the shared half, and it
//! is only ever served *through* an observation the caller already owns — the disclosure policy for
//! the graph is deferred, so nothing here may be reachable except by walking in from a capture.
//!
//! Every field is a projection: `project` is the only writer, so this is a faithful report of what
//! the graph believes rather than something a caller can set.

use serde::{Deserialize, Serialize};

use crate::model;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeEntityInfo {
    pub id: i32,
    pub name: String,
    /// The surfaces the entity is also known by. `name` is not among them.
    pub alt_names: Vec<String>,
    pub entity_type: String,
    pub alt_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub alt_descriptions: Vec<String>,
    /// The disambiguation hint, and the other hints its members carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    pub alt_search_queries: Vec<String>,
    /// The platform presence, when a member showed one. `platform_link` is the wire form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_display_name: Option<String>,
    /// `provisional` · `identified`. A shared identity is not a status: see the proximity record.
    pub status: model::knowledge_entity::Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl KnowledgeEntityInfo {
    pub fn new(entity: model::knowledge_entity::Model) -> Self {
        Self {
            id: entity.id,
            name: entity.name,
            alt_names: entity.alt_names,
            entity_type: entity.entity_type,
            alt_types: entity.alt_types,
            description: entity.description,
            alt_descriptions: entity.alt_descriptions,
            search_query: entity.search_query,
            alt_search_queries: entity.alt_search_queries,
            platform: entity.platform,
            platform_handle: entity.platform_handle,
            platform_display_name: entity.platform_display_name,
            status: entity.status,
            confidence: entity.confidence,
        }
    }
}
