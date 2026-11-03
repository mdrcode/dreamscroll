use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A collision: two entities holding one identity that the graph is forbidden to merge.
///
/// An adjudication queue, not evidence. The rows are a *conclusion* about our own data — the
/// authority says these are one thing, the capture evidence says two, and the same-illumination veto
/// refuses the fold — so a run of `resolve` recomputes them, and deciding one clears it.
///
/// `contested` is derived from this table: an entity is contested while a live row names it as holder.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_collisions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The entity that already holds the identity.
    pub holder_id: i32,
    /// The entity that resolved to the same identity and could not be folded in.
    pub claimant_id: i32,
    /// The identity both of them hold.
    pub knowledge_identity_id: i32,

    /// Why the merge was refused: `shared_illumination` today.
    pub reason: String,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub detected_at: DateTime<Utc>,
    /// Set when a decision settles the pair; the row stops counting while this is null.
    #[sea_orm(nullable)]
    pub adjudicated_at: Option<DateTime<Utc>>,

    #[sea_orm(belongs_to, relation_enum = "Holder", from = "holder_id", to = "id")]
    pub holder: HasOne<knowledge_entity::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "Claimant",
        from = "claimant_id",
        to = "id"
    )]
    pub claimant: HasOne<knowledge_entity::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "Identity",
        from = "knowledge_identity_id",
        to = "id"
    )]
    pub identity: HasOne<knowledge_identity::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
