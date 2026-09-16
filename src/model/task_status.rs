use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

/// One row per task envelope — the canonical source of truth for
/// background-task status. See `_project/plans/sse-task-status.md` §6.
///
/// TODO(REVISIT): the primary read pattern is
/// `query_incomplete_for_entity` — `WHERE user_id = ? AND entity_type = ?
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
#[sea_orm(table_name = "task_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i32,

    /// Globally-unique task identity (see `TaskEnvelope::new`), e.g.
    /// `u1-illuminate-capture123`. Already encodes user_id + task_type + entity.
    #[sea_orm(unique)]
    pub envelope_id: String,

    /// 'illumination' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// The type and id of the entity this task operates on, e.g. "capture" and 42.
    pub entity_type: String,
    #[sea_orm(indexed)]
    pub entity_id: i32,

    /// Integer discriminant of `task::StatusCode`.
    pub status_code: i32,

    pub attempts: i32,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
