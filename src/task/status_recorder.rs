use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::str::FromStr;

use crate::database::DbHandle;
use crate::model::task_status::Status;

/// Encapsulates all direct reads/writes to the `task_status` table.
///
/// This is the single owner of the `task_status` persistence logic, so it can
/// be unit-tested in isolation and reused by `TaskMaster` (writes) and
/// `TaskWatcher` (reads for SSE) without duplicating the SeaORM queries.
#[derive(Clone)]
pub struct TaskStatusRecorder {
    db: Option<DbHandle>,
}

impl TaskStatusRecorder {
    pub fn new(db: Option<DbHandle>) -> Self {
        Self { db }
    }

    /// Record a status transition for a task. Upserts the row keyed by
    /// (task_type, task_id, run_id). `attempts` is the attempt count at the
    /// time of this transition.
    ///
    /// No-op without a DB.
    pub async fn record(
        &self,
        task_type: &str,
        user_id: i32,
        task_id: &str,
        status: Status,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let Some(db) = self.db.as_ref() else {
            return Ok(());
        };

        let existing = crate::model::task_status::Entity::find()
            .filter(crate::model::task_status::Column::TaskType.eq(task_type))
            .filter(crate::model::task_status::Column::TaskId.eq(task_id))
            .filter(crate::model::task_status::Column::RunId.eq(1))
            .one(&db.conn)
            .await?;

        if let Some(row) = existing {
            let mut active: crate::model::task_status::ActiveModel = row.into();
            active.status = Set(status.as_str().to_string());
            active.attempts = Set(attempts);
            active.updated_at = Set(chrono::Utc::now());
            active.update(&db.conn).await?;
        } else {
            crate::model::task_status::ActiveModel::builder()
                .set_task_type(task_type)
                .set_task_id(task_id)
                .set_run_id(1)
                .set_user_id(user_id)
                .set_status(status.as_str().to_string())
                .set_attempts(attempts)
                .set_background(false)
                .save(&db.conn)
                .await?;
        }

        Ok(())
    }

    /// Query the current status row for a task, if one exists.
    ///
    /// Returns `None` when there's no DB or no row yet.
    pub async fn query(
        &self,
        task_type: &str,
        task_id: &str,
    ) -> anyhow::Result<Option<Status>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(None);
        };

        let row = crate::model::task_status::Entity::find()
            .filter(crate::model::task_status::Column::TaskType.eq(task_type))
            .filter(crate::model::task_status::Column::TaskId.eq(task_id))
            .filter(crate::model::task_status::Column::RunId.eq(1))
            .one(&db.conn)
            .await?;

        if let Some(r) = row {
            Ok(Some(Status::from_str(&r.status)?))
        } else {
            Ok(None)
        }
    }
}