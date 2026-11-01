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
    /// provisional | identified | contested. Projected from evidence, never hand-set.
    pub status: String,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,
    /// Set when this entity was merged into another; the loser stays as a tombstone.
    #[sea_orm(nullable)]
    pub merged_into_id: Option<i32>,
    /// Every other name this entity has been seen under: member names, platform display names and
    /// handles. Verbatim, unlike the derived keys the linker builds from them.
    pub alt_names: Vec<String>,
}

impl ActiveModelBehavior for ActiveModel {}
