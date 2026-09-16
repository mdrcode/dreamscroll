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
    /// `envelope_id`. `attempts` is the attempt count at the
    /// time of this transition.
    ///
    /// The full task payload is required: status is only ever recorded by
    /// submitters and workers, both of which hold the concrete task. An
    /// envelope without a payload is a programming error. No-op without a DB.
    pub async fn record<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let Some(db) = self.db.as_ref() else {
            return Ok(());
        };

        let Some(task) = envelope.task.as_ref() else {
            anyhow::bail!(
                "cannot record status for envelope without payload: {}",
                envelope.envelope_id
            );
        };

        let task_type = T::task_type();
        let envelope_id = envelope.envelope_id.as_str();
        let entity_type = T::entity_type();
        let entity_id = task.entity_id();

        let existing = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
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
                .set_envelope_id(envelope_id)
                .set_entity_type(entity_type)
                .set_entity_id(entity_id)
                .set_user_id(envelope.user_id)
                .set_status_code(status.as_i32())
                .set_attempts(attempts)
                .set_background(false)
                .save(&db.conn)
                .await?;
        }

        Ok(())
    }

    /// Query the current status of a single task by its `envelope_id`.
    ///
    /// Returns `None` when there's no DB or no row yet.
    pub async fn query(&self, envelope_id: &str) -> anyhow::Result<Option<StatusCode>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(None);
        };

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .one(&db.conn)
            .await?;

        if let Some(r) = row {
            Ok(Some(StatusCode::from_i32(r.status_code)?))
        } else {
            Ok(None)
        }
    }

    /// Query every task status recorded against a given entity, e.g. all
    /// tasks (`illuminate`, `ingest`, `search_index`, ...) that operate on a
    /// single capture.
    ///
    /// Always scoped by `user_id`: entity ids are not a security boundary, and
    /// callers must never be able to observe another user's task state.
    ///
    /// Returns an empty vec when there's no DB or no matching rows.
    pub async fn query_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(Vec::new());
        };

        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .filter(model::task_status::Column::EntityType.eq(entity_type))
            .filter(model::task_status::Column::EntityId.eq(entity_id))
            .all(&db.conn)
            .await?;

        Ok(rows)
    }
}
