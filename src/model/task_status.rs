use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One row per task envelope — the canonical source of truth for
/// background-task status. See `_project/plans/sse-task-status.md` §6.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i32,

    /// Globally-unique task identity (see `TaskEnvelope::from_task`), e.g.
    /// `u1-illuminate-capture123`. Already encodes user_id + task_type + entity.
    #[sea_orm(unique)]
    pub envelope_id: String,

    /// 'illumination' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// The kind of entity this task operates on, e.g. "capture".
    pub entity_type: String,

    /// The id of the entity this task operates on, e.g. a capture id.
    /// Always present: status is only ever recorded with the full task.
    #[sea_orm(indexed)]
    pub entity_id: i32,

    /// Integer discriminant of `task::StatusCode`.
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
