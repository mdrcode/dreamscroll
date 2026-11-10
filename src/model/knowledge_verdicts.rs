use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// Which statement a verdict row makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Kind {
    /// Two origins are one thing.
    #[sea_orm(string_value = "merge")]
    Merge,
    /// An entity is not an identity. The item stays valid and claimable by everything else.
    #[sea_orm(string_value = "veto_identity")]
    VetoIdentity,
    /// Two entities are deliberately separate. Consultative: it suppresses an automatic fold and
    /// has no effect on the graph by itself.
    #[sea_orm(string_value = "distinct")]
    Distinct,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::VetoIdentity => "veto_identity",
            Self::Distinct => "distinct",
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What decided a merge. The distinction is the point of keeping the log: a person's instruction is
/// replayed in every later generation, the automatic pass's opinion is re-derived, and a replay
/// records that an older repair was applied again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Cause {
    /// The automatic pass folded a proximity candidate. The model's opinion, re-derived every build.
    #[sea_orm(string_value = "proximity_auto")]
    ProximityAuto,
    /// A person decided these are one thing. Kept, and replayed in every later generation.
    #[sea_orm(string_value = "repair")]
    Repair,
    /// An older repair, applied to this generation by the replay.
    #[sea_orm(string_value = "replayed")]
    Replayed,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProximityAuto => "proximity_auto",
            Self::Repair => "repair",
            Self::Replayed => "replayed",
        }
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One merge that happened, and what decided it — the graph's modification log.
///
/// Keyed by `origin_observation_id` on each side, because entity ids are generation-scoped: a reset
/// renumbers entities, and observations are the only thing that survives one. The survivor is
/// recorded rather than just the pair — it is the side that keeps the name, the origin and the
/// projected columns.
///
/// Two kinds share the log, because both are the same statement — a person's decision about the
/// graph, keyed by what survives a rebuild. A `merge` folds two origins into one. A `veto_identity` says an
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
    pub cause: Cause,

    /// `merge` (two origins fold), `veto_identity` (an entity never holds one identity), or
    /// `distinct` (two origins are deliberately separate).
    #[sea_orm(default_value = "merge")]
    pub kind: Kind,

    /// `origin_observation_id` of the side that survives. `merge` only; with `distinct`, the two
    /// origin columns are simply the pair.
    pub winner_origin: Option<i32>,
    /// `origin_observation_id` of the side folded away. `merge` only; with `distinct`, the pair.
    pub loser_origin: Option<i32>,

    /// The entity a `veto_identity` is about, by `origin_observation_id`.
    pub subject_origin: Option<i32>,
    /// The identity a `veto_identity` excludes, by its natural key: a rebuild recreates identity rows, and
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
