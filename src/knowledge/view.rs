//! What the pure strategies see.
//!
//! A `Blocker`, a `Linker` and a `Resolver` never touch a database row. They are handed views: plain
//! values cloned out of the models, with nothing to write and nowhere to reach. That is the whole
//! reason the traits can be synchronous and testable without a database.
//!
//! A view is the **raw** projection — strings exactly as stored. The precomputed forms the scorer
//! and the blocker work with (`Scored`, `score::Observation`) are derived from views, so there is
//! one definition of what an entity is and one place that normalizes it.

use crate::model;

/// One observation, as a strategy sees it. Personal to a user; never rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationView {
    pub id: i32,
    pub capture_id: i32,
    pub illumination_id: i32,
    /// What the capture said, verbatim.
    pub name: String,
    pub entity_type: String,
    pub description: Option<String>,
    /// A platform account the capture showed, if any: `(scheme, handle)`.
    pub account: Option<(String, String)>,
}

impl ObservationView {
    pub fn from_model(row: &model::user_observed_entity::Model) -> Self {
        let account = row
            .platform
            .clone()
            .zip(row.platform_handle.clone())
            .map(|(scheme, handle)| (scheme.to_lowercase(), handle.to_lowercase()));
        Self {
            id: row.id,
            capture_id: row.capture_id,
            illumination_id: row.illumination_id,
            name: row.name.clone(),
            entity_type: row.entity_type.clone(),
            description: row.description.clone(),
            account,
        }
    }
}

/// One knowledge entity, as a strategy sees it. Shared; every field here is a projection of its
/// members and evidence rather than something a caller may disagree with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityView {
    pub id: i32,
    pub name: String,
    /// The selected surface's alternatives. `name` is **not** among them.
    pub alt_names: Vec<String>,
    pub entity_type: String,
    pub alt_types: Vec<String>,
    pub description: Option<String>,
    pub alt_descriptions: Vec<String>,
    /// The best disambiguation hint any member carried.
    pub search_query: Option<String>,
    /// A platform account one of its members carried: `(scheme, handle)`.
    pub account: Option<(String, String)>,
}

impl EntityView {
    /// The row's own fields. `search_query` and `account` come from observations, so a caller that
    /// has them sets them afterwards.
    pub fn from_model(row: model::knowledge_entity::Model) -> Self {
        Self {
            id: row.id,
            name: row.name,
            alt_names: row.alt_names,
            entity_type: row.entity_type,
            alt_types: row.alt_types,
            description: row.description,
            alt_descriptions: row.alt_descriptions,
            search_query: None,
            account: None,
        }
    }

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

    /// Every type its members asserted.
    pub fn types(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.entity_type.as_str()).chain(self.alt_types.iter().map(String::as_str))
    }
}
