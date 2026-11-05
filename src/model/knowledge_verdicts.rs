use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;


/// One merge that happened, and what decided it — the graph's modification log.
///
/// Keyed by `origin_observation_id` on each side, because entity ids are generation-scoped: a reset
/// renumbers entities, and observations are the only thing that survives one. The survivor is
/// recorded rather than just the pair — it is the side that keeps the name, the origin and the
/// projected columns.
///
/// `cause` is the whole point of keeping it: a `repair` is a person's instruction and replay applies
/// it again in every later generation; `proximity_auto` is the model's opinion for one generation and
/// is re-derived, never re-applied; `replayed` records that an older repair was applied again.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_verdicts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The generation the merge happened in.
    pub generation: i32,

    /// `repair` (a person), `proximity_auto` (the automatic pass) or `replayed` (an older repair
    /// applied again).
    #[sea_orm(default_value = "repair")]
    pub cause: String,

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
