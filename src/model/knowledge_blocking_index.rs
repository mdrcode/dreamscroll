use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// Derived retrieval material for one entity: the folded forms a blocker matches on.
///
/// Nothing here is truth. It is a function of `name` and `alt_names`, it is rebuilt wholesale when
/// the fold changes — `built_with` says which fold produced the row — and a missing row must never
/// be read as "this entity has no surfaces". See AGENTS.md §Derived Data.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_blocking_index")]
pub struct Model {
    /// One row per entity, so the entity's own id is the key.
    #[sea_orm(primary_key, auto_increment = false)]
    pub knowledge_entity_id: i32,

    /// Every folded surface, unioned across key generations. A probe is `surfaces && $1`.
    pub surfaces: Vec<String>,

    /// The word-order-insensitive form of each surface. A separate column, not a union with
    /// `surfaces`: an order-only match is a different (weaker) claim about identity, and a channel
    /// that cannot tell which generation matched cannot price it.
    pub word_keys: Vec<String>,

    /// The same surfaces as one folded document, for full text search.
    pub text: String,

    /// The key pipeline's behavioural fingerprint: what rule produced this row, not when.
    #[sea_orm(default_value = "")]
    pub built_with: String,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub built_at: DateTime<Utc>,

    #[sea_orm(belongs_to, from = "knowledge_entity_id", to = "id")]
    pub entity: HasOne<knowledge_entity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
