use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A merge: one entity folded into another.
///
/// A mutation of our own graph, not a fact about the world — the audit trail an unmerge reads, and
/// the only record that a tombstone's pointer means "absorbed here" rather than "deleted".
///
/// `loser_id` is unique: an entity is folded away once, and the tombstone it leaves behind is what
/// makes a repeat a no-op rather than a second merge.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_merges")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The entity that was folded away.
    #[sea_orm(unique)]
    pub loser_id: i32,
    /// The entity that survives.
    pub winner_id: i32,

    /// `identity_conflict` — the resolver found one identity held twice; `repair` — a decision.
    pub cause: String,
    /// Whose decision: `model` or `user`.
    pub source: String,
    #[sea_orm(nullable)]
    pub source_user_id: Option<i32>,
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub at: DateTime<Utc>,

    #[sea_orm(belongs_to, relation_enum = "Loser", from = "loser_id", to = "id")]
    pub loser: HasOne<knowledge_entity::Entity>,
    #[sea_orm(belongs_to, relation_enum = "Winner", from = "winner_id", to = "id")]
    pub winner: HasOne<knowledge_entity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
