use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{database, model};

use super::*;

/// True when a `DbErr` is a unique-constraint violation.
///
/// The `(envelope_id, run)` unique index is the real guard against a duplicate
/// submission, so this case is expected, not exceptional.
fn is_unique_violation(err: &sea_orm::DbErr) -> bool {
    matches!(
        err.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    )
}

/// Encapsulates all direct reads/writes to the `task_status` table.
///
/// This is the single owner of the `task_status` persistence logic, so it can
/// be unit-tested in isolation and reused by `TaskMaster` (writes) and
/// `StatusListener` (reads for SSE) without duplicating the SeaORM queries.
#[derive(Clone)]
pub struct TaskStatusTracker {
    db: database::DbHandle,
}

impl TaskStatusTracker {
    pub fn new(db: database::DbHandle) -> Self {
        Self { db }
    }

    /// Insert the first row for a run. Fails (returns `Ok(false)`) if a row for
    /// this `(envelope_id, run)` already exists.
    ///
    /// `Ok(false)` is the *lost-a-race* case: the caller read the latest run,
    /// decided this run number was free, and a concurrent submit claimed it
    /// first. The composite unique index is what makes correctness rest on the
    /// constraint rather than on the read.
    ///
    /// The full task payload is required: status is only ever recorded by
    /// submitters and workers, both of which hold the concrete task. An
    /// envelope without a payload is a programming error.
    pub async fn create_run<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<bool> {
        let db = &self.db;

        let Some(task) = envelope.task.as_ref() else {
            anyhow::bail!(
                "cannot record status for envelope without payload: {}",
                envelope.envelope_id
            );
        };

        let result = model::task_status::ActiveModel::builder()
            .set_task_type(T::task_type())
            .set_envelope_id(envelope.envelope_id.as_str())
            .set_run(envelope.run)
            .set_entity_type(T::entity_type())
            .set_entity_id(task.entity_id())
            .set_user_id(envelope.user_id)
            .set_status_code(status.as_i32())
            .set_attempts(attempts)
            .save(&db.conn)
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(err) if is_unique_violation(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Update the row for an existing `(envelope_id, run)`.
    ///
    /// A no-op if the row is missing, which can only happen if someone deleted
    /// it mid-flight (we tolerate that rather than resurrecting a row).
    pub async fn update_run<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let db = &self.db;

        model::task_status::Entity::update_many()
            .col_expr(
                model::task_status::Column::StatusCode,
                sea_orm::sea_query::Expr::value(status.as_i32()),
            )
            .col_expr(
                model::task_status::Column::Attempts,
                sea_orm::sea_query::Expr::value(attempts),
            )
            .col_expr(
                model::task_status::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(chrono::Utc::now()),
            )
            .filter(model::task_status::Column::EnvelopeId.eq(envelope.envelope_id.as_str()))
            .filter(model::task_status::Column::Run.eq(envelope.run))
            .exec(&db.conn)
            .await?;

        Ok(())
    }

    /// The most recent run of a logical task, or `None` if it has never run.
    ///
    /// This is the decision input for submit: an in-flight latest run blocks a
    /// new submission, a settled one permits a rerun.
    pub async fn latest_run(
        &self,
        envelope_id: &str,
    ) -> anyhow::Result<Option<model::task_status::Model>> {
        let db = &self.db;

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .order_by_desc(model::task_status::Column::Run)
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// Query the status row for one specific run, used by workers to decide
    /// whether another attempt is warranted.
    ///
    /// Returns `None` when there is no row.
    pub async fn query_run_status(
        &self,
        envelope_id: &str,
        run: i32,
    ) -> anyhow::Result<Option<model::task_status::Model>> {
        let db = &self.db;

        let row = model::task_status::Entity::find()
            .filter(model::task_status::Column::EnvelopeId.eq(envelope_id))
            .filter(model::task_status::Column::Run.eq(run))
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// Query the *incomplete* task statuses recorded against a given entity,
    /// e.g. all tasks (`illuminate`, `search_index`, ...) that
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
    /// Only the **latest run** of each logical task is returned: a rerun
    /// supersedes the run before it, and callers want current state, not a run
    /// history.
    ///
    /// Returns an empty vec when there's no DB or no matching rows.
    pub async fn query_incomplete_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let db = &self.db;

        // TODO(REVISIT): this predicate wants a composite index on
        // (user_id, entity_type, entity_id, status_code). See the note on
        // `model::task_status::Model`.
        //
        // NOTE: deliberately no `status_code` filter here. The latest run must
        // be found among *all* runs — filtering first would let a stale
        // incomplete run shadow a newer completed one. See
        // `incomplete_latest_runs`.
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .filter(model::task_status::Column::EntityType.eq(entity_type))
            .filter(model::task_status::Column::EntityId.eq(entity_id))
            .all(&db.conn)
            .await?;

        Ok(incomplete_latest_runs(rows))
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
    /// failed work (`ErrorExhausted`) is included. Only the latest run of each
    /// logical task is returned.
    ///
    /// Returns an empty vec when there are no matching rows.
    pub async fn query_incomplete_for_user(
        &self,
        user_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        let db = &self.db;

        // TODO(REVISIT): this predicate wants a composite index on
        // (user_id, run). See the note on `model::task_status::Model`.
        let rows = model::task_status::Entity::find()
            .filter(model::task_status::Column::UserId.eq(user_id))
            .all(&db.conn)
            .await?;

        Ok(incomplete_latest_runs(rows))
    }
}

/// Reduce rows to the latest run of each logical task, keeping only those whose
/// latest run is still incomplete.
///
/// The order matters: runs are collapsed **before** the incomplete predicate is
/// applied. Filtering first would let an older incomplete run shadow a newer
/// completed one, reporting work as outstanding when it is actually done.
///
/// Kept in Rust rather than SQL: the filtered set is per-user (or per-entity),
/// which is small, and this avoids a correlated subquery or window function.
fn incomplete_latest_runs(rows: Vec<model::task_status::Model>) -> Vec<model::task_status::Model> {
    let mut latest: std::collections::HashMap<String, model::task_status::Model> =
        std::collections::HashMap::new();

    for row in rows {
        match latest.get(&row.envelope_id) {
            Some(existing) if existing.run >= row.run => {}
            _ => {
                latest.insert(row.envelope_id.clone(), row);
            }
        }
    }

    latest
        .into_values()
        .filter(|row| {
            StatusCode::from_i32(row.status_code)
                .map(|status| status.is_incomplete())
                .unwrap_or(true) // unknown status: surface it rather than hide it
        })
        .collect()
}
