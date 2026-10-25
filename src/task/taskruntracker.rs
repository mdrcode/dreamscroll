use std::sync::Arc;

use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, UpdateMany, sea_query::Expr,
};

use crate::{api, database, model, sse};

use super::statuspublisher::StatusPublisher;
use super::*;

/// Manages all direct reads/writes to `task_runs` in the db.
///
/// Intended to be owned/leveraged by TaskMaster.
///
/// Persistence only: every write funnels through `write_status`, and publication
/// is delegated to `StatusPublisher`.
#[derive(Clone)]
pub struct TaskRunTracker {
    db: database::DbHandle,
    publisher: StatusPublisher,
}

impl TaskRunTracker {
    pub fn new(
        db: database::DbHandle,
        notifier: Option<Arc<dyn sse::ServerEventNotifier>>,
    ) -> Self {
        Self {
            publisher: StatusPublisher::new(db.clone(), notifier),
            db,
        }
    }

    /// Insert a task run in its initial `Queued` state with zero attempts.
    ///
    /// Returns `Ok(false)` for the run ordinal or backfill/logical unique key.
    /// Other uniqueness violations remain errors rather than lost-race outcomes.
    pub async fn insert_run<T: Task>(&self, task_run: &TaskRun<T>) -> anyhow::Result<bool> {
        let task_payload = serde_json::to_value(&task_run.task)?;
        let result = model::task_run::ActiveModel::builder()
            .set_task_type(T::task_type())
            .set_logical_id(task_run.logical_id.as_str())
            .set_run_id(task_run.run_id.as_str())
            .set_run_number(task_run.run_number)
            .set_backfill_id(task_run.backfill_id.clone())
            .set_entity_type(T::entity_type())
            .set_entity_id(task_run.task.entity_id())
            .set_entity_owner_id(task_run.task.entity_owner_id())
            .set_task_payload(Some(task_payload))
            .set_status_code(TaskRunStatus::Queued.as_i32())
            .set_attempts(0)
            .insert(&self.db.conn)
            .await;
        match result {
            Ok(row) => {
                self.publisher.publish(task_run, &row.into()).await;
                Ok(true)
            }
            Err(err) if is_submission_conflict(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Apply one status transition, then publish the resulting row.
    ///
    /// Every status write goes through here, so status, attempts, `updated_at`,
    /// the missing-row outcome, the timing refresh, and publication keep one
    /// order. `extras` adds the columns specific to the transition.
    ///
    /// Timing measures are refreshed for every status; `timing` measures only
    /// `InProgress` and `CompleteSuccess` and ignores the rest.
    async fn write_status<T, F>(
        &self,
        task_run: &TaskRun<T>,
        status: TaskRunStatus,
        attempts: i32,
        extras: F,
    ) -> anyhow::Result<()>
    where
        T: Task,
        F: FnOnce(UpdateMany<model::task_run::Entity>) -> UpdateMany<model::task_run::Entity>,
    {
        let update = extras(
            model::task_run::Entity::update_many()
                .col_expr(
                    model::task_run::Column::StatusCode,
                    Expr::value(status.as_i32()),
                )
                .col_expr(model::task_run::Column::Attempts, Expr::value(attempts))
                .col_expr(
                    model::task_run::Column::UpdatedAt,
                    Expr::current_timestamp(),
                ),
        );

        let rows = update
            .filter(model::task_run::Column::RunId.eq(task_run.run_id.as_str()))
            .exec_with_returning(&self.db.conn)
            .await?;
        let Some(row) = rows.into_iter().next() else {
            tracing::warn!(
                task_run = ?task_run,
                "Ignoring task status update because the run row is missing"
            );
            return Ok(());
        };

        timing::refresh_timing_measures::<T>(&self.db, status).await?;
        self.publisher.publish(task_run, &row).await;
        Ok(())
    }

    /// Set a run's status outside an attempt, optionally recording the failure.
    ///
    /// Submission uses this for a backend that rejects enqueue:
    /// `SubmissionFailed` plus the enqueue error. Attempt outcomes go through
    /// `record_outcome` instead.
    pub async fn set_status<T: Task>(
        &self,
        task_run: &TaskRun<T>,
        status: TaskRunStatus,
        attempts: i32,
        error: Option<&api::ApiError>,
    ) -> anyhow::Result<()> {
        self.write_status(task_run, status, attempts, |update| match error {
            Some(error) => with_error_columns(update, error),
            None => update,
        })
        .await
    }

    /// Mark the most recent attempt as processing and record its start time.
    pub async fn mark_in_progress<T: Task>(
        &self,
        task_run: &TaskRun<T>,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.write_status(task_run, TaskRunStatus::InProgress, attempts, |update| {
            update.col_expr(
                model::task_run::Column::ProcessingStartedAt,
                Expr::current_timestamp(),
            )
        })
        .await
    }

    /// Record an attempt outcome, its duration, and (for failures) the literal
    /// error that was reported.
    ///
    /// `error` is only meaningful for `ErrorWillRetry`/`CompleteFailure`; passing
    /// `None` leaves `last_error_code`/`last_error_message` untouched, which is
    /// what a status-only write wants. A later success does not clear them.
    pub async fn record_outcome<T: Task>(
        &self,
        task_run: &TaskRun<T>,
        status: TaskRunStatus,
        attempts: i32,
        error: Option<&api::ApiError>,
    ) -> anyhow::Result<()> {
        self.write_status(task_run, status, attempts, |update| {
            if status == TaskRunStatus::CompleteSuccess {
                return update.col_expr(
                    model::task_run::Column::SuccessDurationMs,
                    Expr::cust(ATTEMPT_DURATION_MS),
                );
            }
            if !matches!(
                status,
                TaskRunStatus::ErrorWillRetry | TaskRunStatus::CompleteFailure
            ) {
                return update;
            }

            let update = update.col_expr(
                model::task_run::Column::LastErrorDurationMs,
                Expr::cust(ATTEMPT_DURATION_MS),
            );
            match error {
                Some(error) => with_error_columns(update, error),
                None => update,
            }
        })
        .await
    }

    /// Look up a logical task's latest run, or a particular run number, optionally scoped to its target owner.
    pub async fn query_run_status(
        &self,
        logical_id: &str,
        run_number: Option<i32>,
        entity_owner_id: Option<i32>,
    ) -> anyhow::Result<Option<model::task_run::Model>> {
        let mut query = model::task_run::Entity::find()
            .filter(model::task_run::Column::LogicalId.eq(logical_id));
        if let Some(run_number) = run_number {
            query = query.filter(model::task_run::Column::RunNumber.eq(run_number));
        }
        if let Some(entity_owner_id) = entity_owner_id {
            query = query.filter(model::task_run::Column::EntityOwnerId.eq(entity_owner_id));
        }

        Ok(query
            .order_by_desc(model::task_run::Column::RunNumber)
            .one(&self.db.conn)
            .await?)
    }

    /// Look up a logical task already submitted for a backfill, in any state.
    pub async fn query_backfill_run(
        &self,
        backfill_id: &str,
        logical_id: &str,
    ) -> anyhow::Result<Option<model::task_run::Model>> {
        Ok(model::task_run::Entity::find()
            .filter(model::task_run::Column::BackfillId.eq(backfill_id))
            .filter(model::task_run::Column::LogicalId.eq(logical_id))
            .one(&self.db.conn)
            .await?)
    }

    /// Look up one run by its global ID, optionally scoped to its owner.
    pub async fn query_run_by_id(
        &self,
        run_id: &str,
        entity_owner_id: Option<i32>,
    ) -> anyhow::Result<Option<model::task_run::Model>> {
        let mut query =
            model::task_run::Entity::find().filter(model::task_run::Column::RunId.eq(run_id));
        if let Some(entity_owner_id) = entity_owner_id {
            query = query.filter(model::task_run::Column::EntityOwnerId.eq(entity_owner_id));
        }

        Ok(query.one(&self.db.conn).await?)
    }

    /// Latest task statuses per entity and task type for one entity owner.
    pub async fn query_latest_task_runs(
        &self,
        entity_owner_id: i32,
        entity_type: &str,
        entity_ids: &[i32],
    ) -> anyhow::Result<Vec<model::task_run::Model>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }

        Ok(model::task_run::Entity::find()
            .filter(model::task_run::Column::EntityOwnerId.eq(entity_owner_id))
            .filter(model::task_run::Column::EntityType.eq(entity_type))
            .filter(model::task_run::Column::EntityId.is_in(entity_ids.iter().copied()))
            .distinct_on([
                (model::task_run::Entity, model::task_run::Column::EntityId),
                (model::task_run::Entity, model::task_run::Column::TaskType),
            ])
            .order_by_asc(model::task_run::Column::EntityId)
            .order_by_asc(model::task_run::Column::TaskType)
            .order_by_desc(model::task_run::Column::RunNumber)
            .all(&self.db.conn)
            .await?)
    }
}

/// Attempt duration in milliseconds, measured from `begin_attempt`. Yields NULL
/// when no attempt ever started.
const ATTEMPT_DURATION_MS: &str =
    "CAST(EXTRACT(EPOCH FROM (CURRENT_TIMESTAMP - processing_started_at)) * 1000 AS BIGINT)";

/// Cap for a persisted failure message. An upstream error can carry a whole
/// response body in its chain; a bounded row stays readable and cheap to print.
const ERROR_MESSAGE_MAX_CHARS: usize = 2_000;

/// Truncate on a char boundary so the persisted message is always valid UTF-8.
/// The ellipsis counts toward the cap.
fn truncate_error_message(message: String) -> String {
    if message.chars().count() <= ERROR_MESSAGE_MAX_CHARS {
        return message;
    }
    let mut truncated: String = message.chars().take(ERROR_MESSAGE_MAX_CHARS - 1).collect();
    truncated.push('…');
    truncated
}

/// Persist the literal failure report: the failing layer's HTTP status and its
/// annotated error message.
fn with_error_columns(
    update: UpdateMany<model::task_run::Entity>,
    error: &api::ApiError,
) -> UpdateMany<model::task_run::Entity> {
    update
        .col_expr(
            model::task_run::Column::LastErrorCode,
            Expr::value(i32::from(error.status_code.as_u16())),
        )
        .col_expr(
            model::task_run::Column::LastErrorMessage,
            Expr::value(truncate_error_message(error.message())),
        )
}

/// Only expected submission keys arbitrate duplicate work; UUID collisions and
/// any other database constraint violations must still surface as errors.
fn is_submission_conflict(err: &sea_orm::DbErr) -> bool {
    let (sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(error))
    | sea_orm::DbErr::Query(sea_orm::RuntimeErr::SqlxError(error))) = err
    else {
        return false;
    };
    let Some(error) = error.as_database_error() else {
        return false;
    };
    error.code().as_deref() == Some("23505")
        && matches!(
            error.constraint(),
            Some("idx-task_runs-task_run" | "idx-task_runs-backfill_task")
        )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingNotifier(Arc<Mutex<Vec<sse::TaskStatusEvent>>>);

    #[async_trait::async_trait]
    impl sse::ServerEventNotifier for RecordingNotifier {
        async fn notify_task_status(&self, event: &sse::TaskStatusEvent) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct FailingNotifier;

    #[async_trait::async_trait]
    impl sse::ServerEventNotifier for FailingNotifier {
        async fn notify_task_status(&self, _event: &sse::TaskStatusEvent) -> anyhow::Result<()> {
            anyhow::bail!("simulated notification failure")
        }
    }

    /// A minimal task, so tracker tests don't depend on a real task type.
    #[derive(Debug, Clone, serde::Serialize)]
    struct TestTask {
        id: i32,
        entity_owner_id: i32,
    }

    impl Task for TestTask {
        fn task_type() -> &'static str {
            "test"
        }
        fn entity_type() -> &'static str {
            "capture"
        }
        fn entity_id(&self) -> i32 {
            self.id
        }
        fn entity_owner_id(&self) -> i32 {
            self.entity_owner_id
        }
    }

    fn task_run(entity_owner_id: i32, id: i32, run_number: i32) -> TaskRun<TestTask> {
        TaskRun::new(
            TestTask {
                id,
                entity_owner_id,
            },
            run_number,
        )
    }

    fn task_error() -> api::ApiError {
        api::ApiError::internal(anyhow::anyhow!("test failure"))
    }

    async fn insert_run_with_status<T: Task>(
        tracker: &TaskRunTracker,
        task_run: &TaskRun<T>,
        status: TaskRunStatus,
        attempts: i32,
    ) -> anyhow::Result<bool> {
        let created = tracker.insert_run(task_run).await?;
        if created && (status != TaskRunStatus::Queued || attempts != 0) {
            tracker.set_status(task_run, status, attempts, None).await?;
        }
        Ok(created)
    }

    // --- DB-backed ---

    #[tokio::test]
    async fn insert_run_persists_identity_and_task_payload() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = TaskRun::new(
            crate::logic::illuminate::IlluminationTask::new(
                42,
                1,
                "test-model",
                crate::illumination::IlluminationVersion::V2,
            ),
            1,
        );

        assert!(
            tracker
                .insert_run(&env)
                .await
                .expect("insert_run should succeed")
        );

        let stored = tracker
            .query_run_status(&env.logical_id, None, None)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(stored.run_number, 1);
        assert_eq!(stored.entity_owner_id, 1);
        assert_eq!(stored.entity_id, 42);
        assert_eq!(stored.entity_type, "capture");
        assert_eq!(stored.task_type, "illuminate");
        assert_eq!(stored.status_code, TaskRunStatus::Queued.as_i32());
        assert_eq!(
            stored.task_payload,
            Some(serde_json::json!({
                "capture_id": 42,
                "entity_owner_id": 1,
                "model_id": "test-model",
                "prompt_version": "v2",
                "persist_projection": true
            }))
        );
        assert_eq!(stored.attempts, 0);
        assert!(stored.processing_started_at.is_none());
        assert!(stored.last_error_duration_ms.is_none());
        assert!(stored.success_duration_ms.is_none());
    }

    #[tokio::test]
    async fn insert_run_always_starts_queued_with_zero_attempts() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));
        let task_run = task_run(1, 52, 1);

        assert!(tracker.insert_run(&task_run).await.unwrap());

        let stored = tracker
            .query_run_status(&task_run.logical_id, Some(task_run.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status_code, TaskRunStatus::Queued.as_i32());
        assert_eq!(stored.attempts, 0);

        let published = notifier.0.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].payload.status, TaskRunStatus::Queued);
        assert_eq!(published[0].payload.attempts, 0);
    }

    #[tokio::test]
    async fn attempt_timing_records_success_and_retry_failure_durations() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 53, 1);
        assert!(tracker.insert_run(&env).await.unwrap());

        tracker.mark_in_progress(&env, 1).await.unwrap();
        let started = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap()
            .processing_started_at
            .unwrap();
        tracker
            .record_outcome(&env, TaskRunStatus::ErrorWillRetry, 1, Some(&task_error()))
            .await
            .unwrap();
        let after_first_failure = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert!(after_first_failure.processing_started_at.unwrap() >= started);
        assert!(after_first_failure.last_error_duration_ms.unwrap() >= 0);
        assert!(after_first_failure.success_duration_ms.is_none());

        tracker.mark_in_progress(&env, 2).await.unwrap();
        tracker
            .record_outcome(&env, TaskRunStatus::CompleteSuccess, 2, None)
            .await
            .unwrap();
        let completed = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert!(completed.processing_started_at.unwrap() >= started);
        assert!(completed.last_error_duration_ms.unwrap() >= 0);
        assert!(completed.success_duration_ms.unwrap() >= 0);
    }

    #[tokio::test]
    async fn finishing_without_a_processing_start_preserves_null_duration() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 54, 1);
        assert!(tracker.insert_run(&env).await.unwrap());

        tracker
            .record_outcome(&env, TaskRunStatus::CompleteFailure, 1, Some(&task_error()))
            .await
            .unwrap();

        let stored = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status_code, TaskRunStatus::CompleteFailure.as_i32());
        assert!(stored.processing_started_at.is_none());
        assert!(stored.last_error_duration_ms.is_none());
        assert!(stored.success_duration_ms.is_none());
    }

    #[tokio::test]
    async fn failed_attempt_persists_the_literal_reported_error() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 59, 1);
        assert!(tracker.insert_run(&env).await.unwrap());
        tracker.mark_in_progress(&env, 1).await.unwrap();

        let error =
            api::ApiError::internal(anyhow::anyhow!("quota exceeded").context("vertex request"));
        tracker
            .record_outcome(&env, TaskRunStatus::CompleteFailure, 1, Some(&error))
            .await
            .unwrap();

        let stored = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.last_error_code, Some(500));
        assert_eq!(
            stored.last_error_message.as_deref(),
            Some("vertex request: quota exceeded"),
            "the annotated chain is preserved, not reclassified"
        );

        // The report describes the most recent failure; a later success keeps it.
        tracker.mark_in_progress(&env, 2).await.unwrap();
        tracker
            .record_outcome(&env, TaskRunStatus::CompleteSuccess, 2, None)
            .await
            .unwrap();
        let completed = tracker
            .query_run_status(&env.logical_id, Some(env.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            completed.status_code,
            TaskRunStatus::CompleteSuccess.as_i32()
        );
        assert_eq!(completed.last_error_code, Some(500));
        assert_eq!(
            completed.last_error_message.as_deref(),
            Some("vertex request: quota exceeded")
        );
    }

    #[test]
    fn error_message_truncation_keeps_utf8_boundaries() {
        assert_eq!(truncate_error_message("short".to_string()), "short");

        // Three-byte characters: a byte-offset slice at the cap would panic.
        let long = "€".repeat(ERROR_MESSAGE_MAX_CHARS + 5);
        let truncated = truncate_error_message(long);

        assert_eq!(truncated.chars().count(), ERROR_MESSAGE_MAX_CHARS);
        assert!(truncated.starts_with(&"€".repeat(ERROR_MESSAGE_MAX_CHARS - 1)));
        assert!(truncated.ends_with('…'));
    }

    #[tokio::test]
    async fn successful_attempt_refreshes_processing_measure() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 55, 1);
        assert!(tracker.insert_run(&env).await.unwrap());
        tracker.mark_in_progress(&env, 1).await.unwrap();
        tracker
            .record_outcome(&env, TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .unwrap();

        let measure =
            timing::get_timing_estimate(&tracker.db, "test", timing::Measure::ProcessingSuccessful)
                .await
                .expect("successful attempt should create a processing measure");
        assert_eq!(measure.sample_count, 1);
        assert!(measure.duration_ms_avg >= 0);
        assert!(measure.duration_ms_p50 >= 0);
        assert!(measure.duration_ms_p75 >= 0);
        assert!(measure.duration_ms_p90 >= 0);
    }

    #[tokio::test]
    async fn timing_measure_excludes_failures_and_updates_in_place() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        let first = task_run(1, 56, 1);
        assert!(tracker.insert_run(&first).await.unwrap());
        tracker.mark_in_progress(&first, 1).await.unwrap();
        tracker
            .record_outcome(&first, TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .unwrap();

        let failed = task_run(1, 57, 1);
        assert!(tracker.insert_run(&failed).await.unwrap());
        tracker.mark_in_progress(&failed, 1).await.unwrap();
        tracker
            .record_outcome(
                &failed,
                TaskRunStatus::CompleteFailure,
                1,
                Some(&task_error()),
            )
            .await
            .unwrap();

        let measure =
            timing::get_timing_estimate(&tracker.db, "test", timing::Measure::ProcessingSuccessful)
                .await
                .expect("successful processing measure should exist");
        assert_eq!(measure.sample_count, 1, "failures are excluded");

        let timing_row = model::task_run_timing::Entity::find()
            .filter(model::task_run_timing::Column::TaskType.eq("test"))
            .filter(
                model::task_run_timing::Column::OperationType
                    .eq(timing::Measure::ProcessingSuccessful.as_ref()),
            )
            .one(&tracker.db.conn)
            .await
            .unwrap()
            .unwrap();

        let second = task_run(1, 58, 1);
        assert!(tracker.insert_run(&second).await.unwrap());
        tracker.mark_in_progress(&second, 1).await.unwrap();
        tracker
            .record_outcome(&second, TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .unwrap();
        let updated_row = model::task_run_timing::Entity::find()
            .filter(model::task_run_timing::Column::TaskType.eq("test"))
            .filter(
                model::task_run_timing::Column::OperationType
                    .eq(timing::Measure::ProcessingSuccessful.as_ref()),
            )
            .one(&tracker.db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_row.id, timing_row.id, "measure updates in place");
        assert_eq!(updated_row.sample_count, 2);
    }

    #[tokio::test]
    async fn successful_status_writes_publish_events_but_conflicts_and_missing_updates_do_not() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));
        let first_task_run = task_run(1, 42, 1);

        assert!(tracker.insert_run(&first_task_run).await.unwrap());
        assert!(!tracker.insert_run(&task_run(1, 42, 1)).await.unwrap());
        tracker
            .set_status(&first_task_run, TaskRunStatus::InProgress, 1, None)
            .await
            .unwrap();
        tracker
            .set_status(&task_run(1, 99, 1), TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .unwrap();

        let events = notifier.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload.status, TaskRunStatus::Queued);
        assert_eq!(events[1].payload.status, TaskRunStatus::InProgress);
        assert_eq!(events[1].entity_id, 42);
    }

    #[tokio::test]
    async fn notification_failure_does_not_fail_a_status_write() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(FailingNotifier)));
        let task_run = task_run(1, 42, 1);

        assert!(
            tracker
                .insert_run(&task_run)
                .await
                .expect("best-effort notification failure must not fail persistence")
        );
        tracker
            .set_status(&task_run, TaskRunStatus::InProgress, 1, None)
            .await
            .expect("best-effort notification failure must not fail persistence");

        let stored = tracker
            .query_run_status(&task_run.logical_id, Some(task_run.run_number), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status_code, TaskRunStatus::InProgress.as_i32());
    }

    /// The unique index is what actually prevents a duplicate submission, so the
    /// tracker must report the conflict rather than propagate it as an error.
    #[tokio::test]
    async fn insert_run_refuses_a_duplicate_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 42, 1);

        let first = tracker
            .insert_run(&env)
            .await
            .expect("first insert_run should succeed");
        let second = tracker
            .insert_run(&task_run(1, 42, 1))
            .await
            .expect("a conflict must not be an error");

        assert!(first, "the first insert wins");
        assert!(!second, "the second loses the race");
    }

    #[tokio::test]
    async fn backfill_key_rejects_a_stale_claim_at_a_different_run_ordinal() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let mut first = task_run(1, 42, 1);
        first.backfill_id = Some("test-backfill".to_string());
        assert!(tracker.insert_run(&first).await.unwrap());
        tracker
            .set_status(&first, TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .unwrap();

        // A submitter that read "no backfill run" before the winner, but read
        // latest after it completed, can choose the next ordinal. It still loses.
        let mut stale = task_run(1, 42, 2);
        stale.backfill_id = first.backfill_id.clone();
        assert!(!tracker.insert_run(&stale).await.unwrap());
        let stored = tracker
            .query_backfill_run("test-backfill", &first.logical_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.run_id, first.run_id);
        assert_eq!(stored.backfill_id, first.backfill_id);

        // The key is scoped by both backfill and logical task, not either alone.
        let mut other_task = task_run(1, 43, 1);
        other_task.backfill_id = first.backfill_id.clone();
        assert!(tracker.insert_run(&other_task).await.unwrap());
        stale.backfill_id = Some("other-backfill".to_string());
        assert!(tracker.insert_run(&stale).await.unwrap());
        assert!(tracker.insert_run(&task_run(1, 42, 3)).await.unwrap());
        assert!(tracker.insert_run(&task_run(1, 42, 4)).await.unwrap());
    }

    #[tokio::test]
    async fn unrelated_unique_run_id_violation_is_not_a_submission_conflict() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let first = task_run(1, 42, 1);
        assert!(tracker.insert_run(&first).await.unwrap());
        let mut unrelated = task_run(1, 43, 1);
        unrelated.run_id = first.run_id;
        assert!(tracker.insert_run(&unrelated).await.is_err());
    }

    #[tokio::test]
    async fn concurrent_insert_run_calls_are_arbitrated_by_unique_index() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let first = task_run(1, 42, 1);
        let second = task_run(1, 42, 1);

        let (first_created, second_created) =
            tokio::join!(tracker.insert_run(&first), tracker.insert_run(&second),);
        let created = [first_created.unwrap(), second_created.unwrap()];

        assert_eq!(
            created.iter().filter(|was_created| **was_created).count(),
            1,
            "the unique (logical_id, run) constraint must arbitrate concurrent inserts"
        );
    }

    #[tokio::test]
    async fn insert_run_allows_a_new_run_of_the_same_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        assert!(
            insert_run_with_status(
                &tracker,
                &task_run(1, 42, 1),
                TaskRunStatus::CompleteSuccess,
                1,
            )
            .await
            .expect("run 1 should be created")
        );
        assert!(
            tracker
                .insert_run(&task_run(1, 42, 2))
                .await
                .expect("run 2 should be created")
        );

        let latest = tracker
            .query_run_status(&task_run(1, 42, 1).logical_id, None, None)
            .await
            .expect("query should succeed")
            .expect("a row should exist");

        assert_eq!(latest.run_number, 2, "latest_run picks the highest run");
    }

    #[tokio::test]
    async fn insert_run_accepts_the_required_payload() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = task_run(1, 42, 1);
        let result = tracker.insert_run(&env).await;

        assert!(result.is_ok(), "a valid TaskRun carries its task payload");
    }

    #[tokio::test]
    async fn set_status_touches_only_its_own_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let run1 = task_run(1, 42, 1);
        let run2 = task_run(1, 42, 2);

        insert_run_with_status(&tracker, &run1, TaskRunStatus::CompleteFailure, 1)
            .await
            .expect("run 1 should be created");
        tracker
            .insert_run(&run2)
            .await
            .expect("run 2 should be created");

        tracker
            .set_status(&run1, TaskRunStatus::InProgress, 7, None)
            .await
            .expect("update should succeed");

        let stored1 = tracker
            .query_run_status(&run1.logical_id, Some(1), None)
            .await
            .expect("query should succeed")
            .expect("run 1 should exist");
        let stored2 = tracker
            .query_run_status(&run2.logical_id, Some(2), None)
            .await
            .expect("query should succeed")
            .expect("run 2 should exist");

        assert_eq!(stored1.status_code, TaskRunStatus::InProgress.as_i32());
        assert_eq!(stored1.attempts, 7);
        assert_eq!(
            stored2.status_code,
            TaskRunStatus::Queued.as_i32(),
            "run 2 must be untouched"
        );

        // A run that was never written is simply absent, not an error.
        let missing = tracker
            .query_run_status(&run1.logical_id, Some(99), None)
            .await
            .expect("query should succeed");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn query_latest_task_runs_is_owner_scoped() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        tracker
            .insert_run(&task_run(1, 42, 1))
            .await
            .expect("insert_run should succeed");

        let mine = tracker
            .query_latest_task_runs(1, "capture", &[42])
            .await
            .expect("query should succeed");
        let theirs = tracker
            .query_latest_task_runs(2, "capture", &[42])
            .await
            .expect("query should succeed");
        let my_run = tracker
            .query_run_status("u1-test-capture42", Some(1), Some(1))
            .await
            .expect("exact-run query should succeed");
        let other_owners_run = tracker
            .query_run_status("u1-test-capture42", Some(1), Some(2))
            .await
            .expect("exact-run query should succeed");

        assert_eq!(mine.len(), 1);
        assert!(theirs.is_empty(), "entity ids are not a security boundary");
        assert!(my_run.is_some());
        assert!(
            other_owners_run.is_none(),
            "exact-run queries honor the optional owner filter"
        );
    }

    #[tokio::test]
    async fn query_latest_task_runs_filters_and_collapses_runs() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        insert_run_with_status(
            &tracker,
            &task_run(1, 42, 1),
            TaskRunStatus::CompleteFailure,
            2,
        )
        .await
        .expect("older run should be created");
        insert_run_with_status(
            &tracker,
            &task_run(1, 42, 2),
            TaskRunStatus::CompleteSuccess,
            1,
        )
        .await
        .expect("newer run should be created");
        tracker
            .insert_run(&task_run(1, 43, 1))
            .await
            .expect("second entity should be created");
        insert_run_with_status(&tracker, &task_run(2, 42, 1), TaskRunStatus::InProgress, 1)
            .await
            .expect("other user's entity should be created");

        let rows = tracker
            .query_latest_task_runs(1, "capture", &[42, 43, 999])
            .await
            .expect("multi-entity query should succeed");

        assert_eq!(rows.len(), 2, "only matching user/entity rows are returned");
        assert_eq!(
            rows.iter()
                .find(|row| row.entity_id == 42)
                .unwrap()
                .run_number,
            2,
            "only the latest run for an entity is returned"
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.entity_id == 42)
                .unwrap()
                .status_code,
            TaskRunStatus::CompleteSuccess.as_i32()
        );
    }

    #[tokio::test]
    async fn query_latest_task_runs_empty_input_returns_empty() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        let rows = tracker
            .query_latest_task_runs(1, "capture", &[])
            .await
            .expect("empty query should succeed");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn query_latest_task_runs_keeps_distinct_tasks_and_filters_entity_type() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        insert_run_with_status(
            &tracker,
            &task_run(1, 42, 1),
            TaskRunStatus::CompleteSuccess,
            1,
        )
        .await
        .unwrap();
        let search_task = TaskRun::new(
            crate::logic::search_index::SearchIndexTask {
                capture_id: 42,
                entity_owner_id: 1,
            },
            1,
        );
        tracker.insert_run(&search_task).await.unwrap();
        let spark_task = TaskRun::new(
            crate::logic::spark::SparkTask {
                spark_id: 42,
                entity_owner_id: 1,
                capture_ids: vec![1],
            },
            1,
        );
        insert_run_with_status(&tracker, &spark_task, TaskRunStatus::InProgress, 1)
            .await
            .unwrap();

        let capture_rows = tracker
            .query_latest_task_runs(1, "capture", &[42])
            .await
            .unwrap();
        assert_eq!(
            capture_rows.len(),
            2,
            "different logical tasks remain distinct"
        );
        assert!(capture_rows.iter().all(|row| row.entity_type == "capture"));
        assert!(capture_rows.iter().any(|row| row.task_type == "test"));
        assert!(
            capture_rows
                .iter()
                .any(|row| row.task_type == "search_index")
        );

        let spark_rows = tracker
            .query_latest_task_runs(1, "spark", &[42])
            .await
            .unwrap();
        assert_eq!(spark_rows.len(), 1);
        assert_eq!(spark_rows[0].task_type, "spark");
    }

    #[tokio::test]
    async fn latest_run_returns_highest_run_and_none_for_missing_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let first = task_run(1, 42, 1);
        let second = task_run(1, 42, 2);

        insert_run_with_status(&tracker, &first, TaskRunStatus::CompleteFailure, 1)
            .await
            .unwrap();
        tracker.insert_run(&second).await.unwrap();

        assert_eq!(
            tracker
                .query_run_status(&first.logical_id, None, None)
                .await
                .unwrap()
                .unwrap()
                .run_number,
            2
        );
        assert!(
            tracker
                .query_run_status("missing-logical-id", None, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn set_status_for_missing_row_warns_and_does_not_publish() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));

        tracker
            .set_status(&task_run(1, 42, 1), TaskRunStatus::InProgress, 1, None)
            .await
            .expect("missing-run update is ignored after logging a warning");

        assert!(
            tracker
                .query_run_status("u1-test-capture42", None, None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(notifier.0.lock().unwrap().is_empty());
    }

    /// Every status write shares the missing-row outcome; an attempt outcome must
    /// not publish a hint for a run that was never written.
    #[tokio::test]
    async fn record_outcome_for_missing_row_warns_and_does_not_publish() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));

        tracker
            .record_outcome(&task_run(1, 42, 1), TaskRunStatus::CompleteSuccess, 1, None)
            .await
            .expect("missing-run outcome is ignored after logging a warning");

        assert!(
            tracker
                .query_run_status("u1-test-capture42", None, None)
                .await
                .unwrap()
                .is_none()
        );
        assert!(notifier.0.lock().unwrap().is_empty());
    }
}
