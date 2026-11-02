//! What the strategies read, and the keys they derive from it.
//!
//! The strategies take model rows directly — there is no view layer between them and the database
//! shape. What they must not do is *write* one: `project` is the only writer of a derived column,
//! and `link` is the only thing that points an observation at an entity.
//!
//! The methods here are the derived forms the scorer and the blocker work with, so there is one
//! definition of what an entity answers to. They live beside the strategies rather than beside the
//! model so the model file stays a description of the table.

use crate::model;

impl model::knowledge_entity::Model {
    /// Every surface this entity answers to, its selected name first.
    pub fn surfaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.alt_names.iter().map(String::as_str))
    }

    /// The selected description first, then the alternates other members contributed.
    pub fn descriptions(&self) -> impl Iterator<Item = &str> {
        self.description
            .iter()
            .map(String::as_str)
            .chain(self.alt_descriptions.iter().map(String::as_str))
    }

    /// Every type its members asserted, the selected one first.
    pub fn types(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.entity_type.as_str()).chain(self.alt_types.iter().map(String::as_str))
    }
}

/// A knowledge entity that exists only in memory: the fields a strategy reads, and nothing else.
///
/// For tests and the offline evaluation example, where a full database row would be seventeen lines
/// of noise. Never used to write.
pub fn scratch(id: i32, name: &str, entity_type: &str) -> model::knowledge_entity::Model {
    let now = chrono::Utc::now();
    model::knowledge_entity::Model {
        id,
        created_at: now,
        updated_at: now,
        name: name.to_string(),
        alt_names: Vec::new(),
        entity_type: entity_type.to_string(),
        alt_types: Vec::new(),
        description: None,
        alt_descriptions: Vec::new(),
        search_query: None,
        alt_search_queries: Vec::new(),
        platform: None,
        platform_handle: None,
        platform_display_name: None,
        status: "provisional".to_string(),
        confidence: None,
        merged_into_id: None,
    }
}

/// The same, for one observation.
pub fn scratch_observation(
    id: i32,
    name: &str,
    entity_type: &str,
) -> model::user_observed_entity::Model {
    model::user_observed_entity::Model {
        id,
        user_id: 1,
        capture_id: 1,
        illumination_id: 1,
        position: 0,
        created_at: chrono::Utc::now(),
        name: name.to_string(),
        entity_type: entity_type.to_string(),
        description: None,
        search_query: None,
        platform: None,
        platform_handle: None,
        platform_display_name: None,
        knowledge_entity_id: None,
    }
}
