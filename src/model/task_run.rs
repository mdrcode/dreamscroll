use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// Persistence representation of a TaskRun. In addition to its identity and
/// task payload, each row stores mutable lifecycle and timing state.
/// See `plan/archive/task-status.md` §3 and §6.
///
/// A logical task (`logical_id`) can be run more than once; each invocation gets
/// its own row, numbered from 1. `(logical_id, run_number)` is unique, preventing
/// concurrent submissions from claiming the same ordinal. Tagged runs also have
/// a unique `(backfill_id, logical_id)` index, independent of their outcome.
///
/// TODO(REVISIT): `query_latest_task_runs` filters by entity owner, entity type,
/// and entity IDs, then orders by `entity_id`, `task_type`, and `run_number DESC`.
/// Currently only the single-column `entity_id` index supports the query. Add a
/// composite index on `(entity_owner_id, entity_type, entity_id, task_type,
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

    /// Identifies the logical task (not one of its runs), e.g.
    /// `u1-illuminate-capture123`.
    #[sea_orm(unique_key = "task_run")]
    pub logical_id: String,

    /// Globally unique UUID for this TaskRun.
    /// Task-owned raw or partial results use this ID as their stable key across retries.
    #[sea_orm(unique)]
    pub run_id: String,

    /// 1-based ordinal among runs of the same logical task.
    #[sea_orm(unique_key = "task_run")]
    pub run_number: i32,

    /// Backfill correlation; null for ordinary submissions and preexisting runs.
    pub backfill_id: Option<String>,

    /// 'illuminate' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// Owner of the target entity, not the user who submitted the task.
    pub entity_owner_id: i32,
    /// The type and id of the entity this task operates on, e.g. "capture" and 42.
    pub entity_type: String,
    pub entity_id: i32,

    /// Full serialized Task parameters for this run; null for rows created before this field.
    #[sea_orm(nullable, column_type = "JsonBinary")]
    pub task_payload: Option<Json>,

    /// Integer discriminant of `task::TaskRunStatus`.
    pub status_code: i32,

    pub attempts: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    /// When the most recent attempt entered `InProgress`.
    pub processing_started_at: Option<DateTime<Utc>>,

    /// Most specific HTTP status known for the failure, stored verbatim.
    ///
    /// When the failure originated in an upstream provider response, this is the
    /// provider's status (for example 429), even though the failing route returns
    /// 500 to its caller. Deliberately not a taxonomy. See
    /// `plan/archive/task-status.md` §10.
    pub last_error_code: Option<i32>,

    /// Message of the most recent failed attempt: the annotated error chain on
    /// a single line, truncated by `TaskRunTracker`. Null for runs that never
    /// failed, and preserved (not cleared) by a later successful attempt.
    pub last_error_message: Option<String>,

    /// Processing duration of the most recent failed attempt, in milliseconds.
    pub last_error_duration_ms: Option<i64>,

    /// Processing duration of the successful attempt, in milliseconds.
    pub success_duration_ms: Option<i64>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}

/// A complete, valid row for tests that assert on a subset of the columns: take
/// this and mutate the fields the test cares about, so adding a column does not
/// require editing every fixture.
#[cfg(test)]
pub fn task_run_model() -> Model {
    Model {
        id: 1,
        entity_owner_id: 1,
        logical_id: "u1-illuminate-capture42".to_string(),
        run_id: "8a0d329d-72ca-4fd5-bd8d-2302f762f37d".to_string(),
        run_number: 1,
        backfill_id: None,
        task_type: "illuminate".to_string(),
        entity_type: "capture".to_string(),
        entity_id: 42,
        task_payload: None,
        status_code: 1, // Queued
        attempts: 0,
        created_at: chrono::Utc::now(),
        processing_started_at: None,
        last_error_code: None,
        last_error_message: None,
        last_error_duration_ms: None,
        success_duration_ms: None,
        updated_at: chrono::Utc::now(),
    }
}
