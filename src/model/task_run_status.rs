use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One row per task **run** — the canonical source of truth for
/// background task runs. See `plan/task-status.md` §3 and §6.
///
/// A logical task (`envelope_id`) can be run more than once; each run gets its
/// own row, numbered from 1. `(envelope_id, run)` is unique, which is what
/// prevents a duplicate submission of work that is still in flight.
///
/// TODO(REVISIT): the primary read pattern is
/// `query_latest_status_for_entities` — `WHERE user_id = ? AND entity_type = ?
/// AND entity_id = ? AND status_code IN (...)` — which currently only has the
/// single-column `entity_id` index to work with. Add a composite index on
/// `(user_id, entity_type, entity_id, status_code)` once the table is large
/// enough to matter. Note SeaORM's derive only supports single-column
/// `#[sea_orm(indexed)]` and composite `unique_key`, so a non-unique composite
/// index needs raw SQL (e.g. `CREATE INDEX ... IF NOT EXISTS` alongside the
/// schema sync in `database/postgres.rs`). Deferred deliberately: this is a
/// single-user app and the table is tiny.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_run_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i32,

    /// Identifies the *logical* task (not the run), e.g.
    /// `u1-illuminate-capture123`. Already encodes user_id + task_type + entity.
    #[sea_orm(unique_key = "task_run")]
    pub envelope_id: String,

    /// Which run of the logical task this row records, counting up from 1.
    /// A rerun of completed work creates a new row with the next number.
    #[sea_orm(unique_key = "task_run")]
    pub run: i32,

    /// 'illuminate' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// The type and id of the entity this task operates on, e.g. "capture" and 42.
    pub entity_type: String,
    #[sea_orm(indexed)]
    pub entity_id: i32,

    /// Integer discriminant of `task::TaskRunStatus`.
    pub status_code: i32,

    pub attempts: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
