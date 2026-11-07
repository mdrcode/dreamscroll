use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;


/// One merge that happened, and what decided it — the graph's modification log.
///
/// Keyed by `origin_observation_id` on each side, because entity ids are generation-scoped: a reset
/// renumbers entities, and observations are the only thing that survives one. The survivor is
/// recorded rather than just the pair — it is the side that keeps the name, the origin and the
/// projected columns.
///
/// Two kinds share the log, because both are the same statement — a person's decision about the
/// graph, keyed by what survives a rebuild. A `merge` folds two origins into one. A `veto` says an
/// entity is *not* an identity: the item stays valid and claimable by everything else.
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

    /// `merge` (two origins fold) or `veto` (an entity never holds one identity).
    #[sea_orm(default_value = "merge")]
    pub kind: String,

    /// `origin_observation_id` of the side that survives. `merge` only.
    pub winner_origin: Option<i32>,
    /// `origin_observation_id` of the side folded away. `merge` only.
    pub loser_origin: Option<i32>,

    /// The entity a `veto` is about, by `origin_observation_id`.
    pub subject_origin: Option<i32>,
    /// The identity a `veto` excludes, by its natural key: a rebuild recreates identity rows, and
    /// `(scheme, external_id)` is the only stable name they have.
    pub identity_scheme: Option<String>,
    pub identity_external_id: Option<String>,

    pub winner_label: Option<String>,
    pub loser_label: Option<String>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub decided_at: DateTime<Utc>,

    pub note: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
