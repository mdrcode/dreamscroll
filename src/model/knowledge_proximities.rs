use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A proximity: two entities the graph considers close, and what was decided about them.
///
/// A record, not a work queue. It says *why* the pair was raised (`basis`), how close they looked
/// (`score`), and what happened (`verdict`) — and rows are never deleted, because "these two turned
/// out to be one thing" is the most useful row in the table.
///
/// `verdict` null means nobody has decided. An entity with an open row is `contested`: it shares an
/// identity with another entity and the graph has not settled which of them it is.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_proximities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The entity the graph met first, or the more substantiated of the two. Direction is a
    /// convention for reading, not a claim about which is right.
    pub holder_id: i32,
    /// The other one.
    pub counterpart_id: i32,

    /// The identity they share. Null for a proximity raised by something other than an identity
    /// — similar names and types, say — which is what `basis` distinguishes.
    #[sea_orm(nullable)]
    pub knowledge_identity_id: Option<i32>,

    /// Why the pair was raised: `shared_id` today.
    pub basis: String,
    /// How close they looked when raised — the linker's weighted total, so the table can be read in
    /// likelihood order. Null for a pair recorded without scoring.
    #[sea_orm(nullable)]
    pub score: Option<f32>,

    /// `NULL` open | `merged` folded together | `distinct` a person said two things | `moot` one side
    /// was folded away by some other path.
    #[sea_orm(nullable)]
    pub verdict: Option<String>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub raised_at: DateTime<Utc>,
    #[sea_orm(nullable)]
    pub settled_at: Option<DateTime<Utc>>,

    #[sea_orm(belongs_to, relation_enum = "Holder", from = "holder_id", to = "id")]
    pub holder: HasOne<knowledge_entity::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "Counterpart",
        from = "counterpart_id",
        to = "id"
    )]
    pub counterpart: HasOne<knowledge_entity::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "Identity",
        from = "knowledge_identity_id",
        to = "id"
    )]
    pub identity: HasOne<knowledge_identity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
