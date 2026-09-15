use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::{database, model};

use super::*;

/// Encapsulates all direct reads/writes to the `task_status` table.
///
/// This is the single owner of the `task_status` persistence logic, so it can
/// be unit-tested in isolation and reused by `TaskMaster` (writes) and
/// `TaskWatcher` (reads for SSE) without duplicating the SeaORM queries.
#[derive(Clone)]
pub struct TaskStatusRecorder {
    db: Option<database::DbHandle>,
}

impl TaskStatusRecorder {
    pub fn new(db: Option<database::DbHandle>) -> Self {
        Self { db }
    }

    /// Record a status transition for a task. Upserts the row keyed by
    /// (task_type, task_id). `attempts` is the attempt count at the
    /// time of this transition.
    ///
    /// `task_type`, `user_id`, and `task_id` are all derived from the
    /// `TaskEnvelope`. No-op without a DB.
    pub async fn record<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let Some(db) = self.db.as_ref() else {
            return Ok(());
        };

        let task_type = T::task_type();
        let task_id = envelope.task_id.as_str();

        let existing = model::task_status::Entity::find()
            .filter(model::task_status::Column::TaskType.eq(task_type))
            .filter(model::task_status::Column::TaskId.eq(task_id))
            .one(&db.conn)
            .await?;

        if let Some(row) = existing {
            let mut active: model::task_status::ActiveModel = row.into();
            active.status_code = Set(status.as_i32());
            active.attempts = Set(attempts);
            active.updated_at = Set(chrono::Utc::now());
            active.update(&db.conn).await?;
        } else {
            model::task_status::ActiveModel::builder()
                .set_task_type(task_type)
                .set_task_id(task_id)
                .set_user_id(envelope.user_id)
                .set_status_code(status.as_i32())
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
    ) -> anyhow::Result<Option<StatusCode>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(None);
        };

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::TaskType.eq(task_type))
            .filter(model::task_status::Column::TaskId.eq(task_id))
            .one(&db.conn)
            .await?;

        if let Some(r) = row {
            Ok(Some(StatusCode::from_i32(r.status_code)?))
        } else {
            Ok(None)
        }
    }
}
