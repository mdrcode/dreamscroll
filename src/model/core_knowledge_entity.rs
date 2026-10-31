use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One canonical referent. Owned by nobody: `status` records how well substantiated it is, and the
/// observations that point here are what say who captured it. External identities live in
/// `core_knowledge_identifiers` — an authority is a namespace, not a column. See
/// `plan/knowledge-canonical-entities.md`.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "core_knowledge_entities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,

    /// Display surface. Its normalized form is always present in `aliases`.
    pub label: String,
    pub entity_type: String,
    #[sea_orm(nullable)]
    pub description: Option<String>,
    /// Where `description` came from: wikidata | wikipedia | model | capture.
    #[sea_orm(nullable)]
    pub description_source: Option<String>,
    /// provisional | identified | contested. Projected from evidence, never hand-set.
    pub status: String,
    #[sea_orm(nullable)]
    pub confidence: Option<f32>,
    /// Set when this row was merged into another; the loser stays as a tombstone.
    #[sea_orm(nullable)]
    pub superseded_by_id: Option<i32>,
    /// Normalized surfaces: authority names plus every observed name and platform display name.
    pub aliases: Vec<String>,
}

impl ActiveModelBehavior for ActiveModel {}
