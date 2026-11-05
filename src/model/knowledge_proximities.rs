use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::*;

/// A candidate: two entities the last scan found close, and why.
///
/// The table is one scan's output, not a ledger — `scan` replaces it wholesale, so a row means "these
/// two looked like one thing when the graph was last asked". No status, no verdict, no history: a
/// pair that keeps looking like one thing keeps reappearing, and one that stops, stops.
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

    /// The identity they share. Null for a candidate raised by something other than an identity.
    #[sea_orm(nullable)]
    pub knowledge_identity_id: Option<i32>,

    /// Why the pair was raised: `shared_id` today.
    pub basis: String,
    /// How close they looked at this scan — the linker's weighted total, so the batch reads in
    /// likelihood order. Null for a pair recorded without scoring.
    #[sea_orm(nullable)]
    pub score: Option<f32>,

    /// When the scan that produced this row ran. Repeated per row rather than held in a scan table.
    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub scanned_at: DateTime<Utc>,

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
