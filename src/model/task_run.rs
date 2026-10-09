use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// Persistence representation of a TaskRun. In addition to its identity and
/// task payload, each row stores mutable lifecycle, timing, and result state.
/// See `plan/task-status.md` §3 and §6.
///
/// A logical task (`logical_id`) can be run more than once; each invocation gets
/// its own row, numbered from 1. `(logical_id, run_number)` is unique, preventing
/// duplicate submission of work that is still in flight.
///
/// TODO(REVISIT): `query_latest_task_runs` filters by user, entity type, and
/// entity IDs, then orders by `entity_id`, `task_type`, and `run_number DESC`.
/// Currently only the single-column `entity_id` index supports the query. Add a
/// composite index on `(user_id, entity_type, entity_id, task_type,
/// run_number DESC)` once the table is large enough to matter. SeaORM's derive
/// only supports single-column `#[sea_orm(indexed)]` and composite `unique_key`,
/// so this needs raw SQL alongside schema sync. Deferred deliberately: this is
/// a single-user app and the table is tiny.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i32,

    /// Identifies the logical task (not one of its runs), e.g.
    /// `u1-illuminate-capture123`.
    #[sea_orm(unique_key = "task_run")]
    pub logical_id: String,

    /// Globally unique UUID for this TaskRun.
    #[sea_orm(unique)]
    pub run_id: String,

    /// 1-based ordinal among runs of the same logical task.
    #[sea_orm(unique_key = "task_run")]
    pub run_number: i32,

    /// 'illuminate' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// The type and id of the entity this task operates on, e.g. "capture" and 42.
    pub entity_type: String,
    // TODO: Restore this index annotation when schema synchronization avoids
    // reissuing redundant CREATE INDEX statements on every startup.
    pub entity_id: i32,

    /// Full serialized Task parameters for this run; null for rows created before this field.
    #[sea_orm(nullable, column_type = "JsonBinary")]
    pub task_payload: Option<Json>,

    #[sea_orm(nullable)]
    pub result_entity_type: Option<String>,
    #[sea_orm(nullable)]
    pub result_entity_id: Option<String>,

    /// Integer discriminant of `task::TaskRunStatus`.
    pub status_code: i32,

    pub attempts: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    /// When the most recent attempt entered `InProgress`.
    pub processing_started_at: Option<DateTime<Utc>>,

    /// Processing duration of the most recent failed attempt, in milliseconds.
    pub last_error_duration_ms: Option<i64>,

    /// Processing duration of the successful attempt, in milliseconds.
    pub success_duration_ms: Option<i64>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
