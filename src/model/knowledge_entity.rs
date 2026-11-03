use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One knowledge entity: the thing itself, shared by every user.
///
/// Owned by nobody. `status` records how well substantiated it is, and the observations pointing
/// here are what say who captured it. `name`, `alt_names`, `entity_type`, `alt_types`,
/// `description`, `alt_descriptions`, `status` and `confidence` are all a projection of the
/// members and the evidence — nothing else writes them. See `plan/lexicon.md`.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_entities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,

    /// The surface chosen to represent this entity. Never one of `alt_names`.
    pub name: String,
    pub entity_type: String,
    /// Every other type its observations asserted. Mostly synonymy in practice — `brand` beside
    /// `organization` — so a matcher accepts agreement with any of them rather than forcing the
    /// taxonomy to choose.
    pub alt_types: Vec<String>,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    /// Every other description supplied. `description` is the selected one; these are what a
    /// matcher may also fit a candidate against.
    pub alt_descriptions: Vec<String>,

    /// The canonical disambiguation hint, projected from its members exactly as `description` is.
    /// Kept even though nothing consumes it yet: it is the phrasing a search engine needs, and it
    /// would be a shame to lose it and have to reacquire it from the illuminator.
    #[sea_orm(nullable)]
    pub search_query: Option<String>,
    /// Every other hint a member carried. `search_query` is the selected one.
    pub alt_search_queries: Vec<String>,
    /// provisional | identified | contested. Projected from evidence, never hand-set.
    pub status: String,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,
    /// Set when this entity was merged into another; the loser stays as a tombstone.
    #[sea_orm(nullable)]
    pub merged_into_id: Option<i32>,
    /// The member whose values this entity wears: the observation it was created from.
    ///
    /// Survives a merge, so the winner keeps its own surface and text instead of inheriting the
    /// loser's. Declared without a foreign key — the constraint would be a cycle with
    /// `user_observed_entities.knowledge_entity_id`, and observations are never deleted.
    #[sea_orm(nullable)]
    pub origin_observation_id: Option<i32>,
    /// Every other name this entity has been seen under: member names, platform display names and
    /// handles. Verbatim, unlike the derived keys the linker builds from them.
    pub alt_names: Vec<String>,

    /// The platform presence, copied from a member by `project`. Flat rather than a link table:
    /// one presence per entity is all the resolver needs, and a member can have at most one.
    #[sea_orm(nullable)]
    pub platform: Option<String>,
    #[sea_orm(nullable)]
    pub platform_handle: Option<String>,
    #[sea_orm(nullable)]
    pub platform_display_name: Option<String>,

    // if this is non-null, the entity is a tombstone (it has been merge'd into another entity)
    #[sea_orm(belongs_to, from = "merged_into_id", to = "id")]
    pub merged_into: HasOne<crate::model::knowledge_entity::Entity>,
}

impl Model {
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

impl ActiveModelBehavior for ActiveModel {}
