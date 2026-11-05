use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;


/// A person's decision that two entities are one thing, kept so a rebuild does not lose it.
///
/// Keyed by `origin_observation_id` on each side, because entity ids are generation-scoped: a reset
/// renumbers entities, and observations are the only thing that survives one. The survivor is
/// recorded rather than just the pair — it is the side that keeps the name, the origin and the
/// projected columns.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_verdicts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The generation the person decided in. Provenance, not a gate: replay applies any age.
    pub generation: i32,

    /// `origin_observation_id` of the side that survives.
    pub winner_origin: i32,
    /// `origin_observation_id` of the side folded away.
    pub loser_origin: i32,

    pub winner_label: Option<String>,
    pub loser_label: Option<String>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub decided_at: DateTime<Utc>,

    pub note: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
