use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One build cycle. `reset` closes it and opens the next.
///
/// The row doubles as the snapshot register: reset is the only snapshotter and it empties as it
/// copies, so at most one snapshot belongs to a generation and there is no second table to keep.
/// `snapshot` is null when the graph was empty, which is also the only case with no archive tables.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "knowledge_generations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub started_at: DateTime<Utc>,

    /// Set by reset: the moment this generation was snapshotted and emptied.
    #[sea_orm(nullable)]
    pub superseded_at: Option<DateTime<Utc>>,

    /// Archive-table suffix, `20261105T004503900`.
    #[sea_orm(nullable)]
    pub snapshot: Option<String>,

    pub entities: Option<i32>,
    pub identities: Option<i32>,
    pub merges: Option<i32>,
    pub proximities: Option<i32>,
    pub members: Option<i32>,
}

impl ActiveModelBehavior for ActiveModel {}
