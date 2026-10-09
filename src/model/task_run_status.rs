use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One row per task **run** — the canonical source of truth for
/// background task runs. See `plan/task-status.md` §3 and §6.
///
/// A logical task (`logical_id`) can be run more than once; each run gets its
/// own row, numbered from 1. `(logical_id, run)` is unique, which is what
/// prevents a duplicate submission of work that is still in flight.
///
/// TODO(REVISIT): the primary read pattern is
/// `query_latest_task_runs` — `WHERE user_id = ? AND entity_type = ?
/// AND entity_id IN (...) ORDER BY entity_id, task_type, run DESC` — which
/// currently only has the single-column `entity_id` index to work with. Add a
/// composite index on `(user_id, entity_type, entity_id, task_type, run DESC)`
/// once the table is large enough to matter. Note SeaORM's derive only supports
/// single-column `#[sea_orm(indexed)]` and composite `unique_key`, so a non-unique
/// composite index needs raw SQL (e.g. `CREATE INDEX ... IF NOT EXISTS` alongside
/// the schema sync in `database/postgres.rs`). Deferred deliberately: this is a
/// single-user app and the table is tiny.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_run_status")]
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
