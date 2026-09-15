use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One row per (task_type, task_id) — the canonical source of truth
/// for background-task status. See `_project/plans/sse-task-status.md` §6.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,

    /// 'illumination' | 'spark' | 'search_index' | ...
    pub task_type: String,
    pub user_id: i32,
    pub task_id: String,

    /// Integer discriminant of `task::Status` (see `task::task_status`).
    pub status_code: i32,

    pub attempts: i32,

    /// true for backfill/bulk tasks (metadata only, not a filter — see §4.3).
    pub background: bool,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
