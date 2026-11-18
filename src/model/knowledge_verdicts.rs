use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use crate::knowledge::mutation::Cause;

/// Which statement a verdict row makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::None)")]
pub enum Kind {
    /// Two origins are one thing.
    #[sea_orm(string_value = "merge")]
    Merge,
    /// The annotation is not evidence for this entity (a resolver false positive). The item stays
    /// valid and claimable by everything else.
    #[sea_orm(string_value = "veto_identity")]
    VetoIdentity,
    /// Two entities are deliberately separate, even when one canonical document covers both (an
    /// album and its songs). Consultative: it suppresses an automatic fold and has no effect on the
    /// graph by itself.
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

/// One merge that happened, and what decided it — the graph's modification log.
///
/// Keyed by `origin_observation_id` on each side, because entity ids are generation-scoped: a reset
/// renumbers entities, and observations are the only thing that survives one. The survivor is
/// recorded rather than just the pair — it is the side that keeps the name, the origin and the
/// projected columns.
///
/// Three kinds share the log, because each is the same statement — a decision about the graph,
/// keyed by what survives a rebuild. A `merge` folds two origins into one. A `veto_identity` says an
/// entity is *not* an identity: the item stays valid and claimable by everything else. A `distinct`
/// keeps two origins separate despite a shared identity.
///
/// `cause` is the whole point of keeping it: a `repair` is a person's instruction and replay applies
/// it again in every later generation; a `model` decision is durable but not a person's.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_verdicts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    /// The generation the merge happened in.
    pub generation: i32,

    /// `repair` (a person) or `model` (a model).
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
