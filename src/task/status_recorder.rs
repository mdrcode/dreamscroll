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

/// A point-in-time view of a task's status row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskStatusSnapshot {
    pub status: StatusCode,
    pub attempts: i32,
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
                .save(&db.conn)
                .await?;
        }

        Ok(())
    }

    /// Query the status *and* current attempt count for a task, used by
    /// workers to decide whether another attempt is warranted.
    ///
    /// Returns `None` when there's no DB or no row yet.
    pub async fn query_snapshot(
        &self,
        envelope_id: &str,
    ) -> anyhow::Result<Option<TaskStatusSnapshot>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(None);
        };

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .one(&db.conn)
            .await?;

        match row {
            Some(r) => Ok(Some(TaskStatusSnapshot {
                status: StatusCode::from_i32(r.status_code)?,
                attempts: r.attempts,
            })),
            None => Ok(None),
        }
    }

    /// Query the *incomplete* task statuses recorded against a given entity,
    /// e.g. all tasks (`illuminate`, `ingest`, `search_index`, ...) that
    /// operate on a single capture and have not yet succeeded.
    ///
    /// Incomplete means everything except `Completed`: in-flight tasks
    /// (`Queued`, `InProgress`, `ErrorWillRetry`) *and* `ErrorExhausted`, since
    /// work that permanently failed is still something the user wants to see.
    /// Completed rows are vacuumed over time, so this API deliberately cannot
    /// express "give me everything" — that would silently return an incomplete
    /// history.
    ///
    /// Always scoped by `user_id`: entity ids are not a security boundary, and
    /// callers must never be able to observe another user's task state.
    ///
    /// Returns an empty vec when there's no DB or no matching rows.
    pub async fn query_incomplete_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(Vec::new());
        };

        // TODO(REVISIT): this predicate wants a composite index on
        // (user_id, entity_type, entity_id, status_code). See the note on
        // `model::task_status::Model`.
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .filter(model::task_status::Column::EntityType.eq(entity_type))
            .filter(model::task_status::Column::EntityId.eq(entity_id))
            .filter(model::task_status::Column::StatusCode.is_in(StatusCode::incomplete_codes()))
            .all(&db.conn)
            .await?;

        Ok(rows)
    }

    /// Query every *incomplete* task status for a user, across all entities.
    ///
    /// This is the user-level counterpart to `query_incomplete_for_entity`:
    /// useful when the caller wants every outstanding task a user has (e.g. a
    /// global progress indicator or a "what's still running?" view) rather than
    /// the tasks for one specific entity.
    ///
    /// "Incomplete" means everything except `Completed` (see
    /// `query_incomplete_for_entity` for the full rationale), so permanently
    /// failed work (`ErrorExhausted`) is included.
    ///
    /// Returns an empty vec when there's no DB or no matching rows.
    pub async fn query_incomplete_for_user(
        &self,
        user_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(Vec::new());
        };

        // TODO(REVISIT): this predicate wants a composite index on
        // (user_id, status_code). See the note on `model::task_status::Model`.
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .filter(model::task_status::Column::StatusCode.is_in(StatusCode::incomplete_codes()))
            .all(&db.conn)
            .await?;

        Ok(rows)
    }
}
