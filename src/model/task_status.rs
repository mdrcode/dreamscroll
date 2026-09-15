use anyhow::anyhow;
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Typed status values stored in the `status` TEXT column.
///
/// Mirrors `task::Status` but is self-contained here so the model doesn't
/// depend on the task module. Keep the string forms in sync with the DB
/// values used by the workers (`queued|in_progress|completed|error|error_final`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    InProgress,
    Completed,
    Error,
    ErrorFinal,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::InProgress => "in_progress",
            Status::Completed => "completed",
            Status::Error => "error",
            Status::ErrorFinal => "error_final",
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Status {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Status::Queued),
            "in_progress" => Ok(Status::InProgress),
            "completed" => Ok(Status::Completed),
            "error" => Ok(Status::Error),
            "error_final" => Ok(Status::ErrorFinal),
            other => Err(anyhow!("unknown task status: {other}")),
        }
    }
}

/// One row per (task_type, task_id, run_id) — the canonical source of truth
/// for background-task status. See `_project/plans/sse-task-status.md` §6.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "task_status")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,

    /// 'illumination' | 'spark' | 'search_index' | ...
    pub task_type: String,

    /// capture_id (single) or capture_ids joined (bulk/backfill).
    pub task_id: String,

    /// Increments on rerun; part of the uniqueness key.
    pub run_id: i64,

    pub user_id: i32,

    /// One of `Status`'s string forms.
    pub status: String,

    pub attempts: i32,

    /// true for backfill/bulk tasks (metadata only, not a filter — see §4.3).
    pub background: bool,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub created_at: DateTime<Utc>,

    #[sea_orm(default_expr = "Expr::current_timestamp()")]
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}
