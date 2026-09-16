use crate::api;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::model;

use super::*;

/// The primary entry point for manipulating `Task` instances.
///
/// `TaskMaster` owns the backend queues **and** the `task_status` table. It is
/// one of only two structs allowed to touch `task_status` directly (the other
/// is `StatusListener`, the future LISTEN/NOTIFY thread). Everything else in the
/// system talks to tasks through this API:
///
/// - `submit_*` — enqueue + record a `Queued` row.
/// - `begin_attempt` / `finish_attempt` — the worker-side attempt lifecycle.
/// - `query_*` — read status (replay / polling).
///
/// Status transitions are deliberately *not* exposed as a raw setter: workers
/// must go through `begin_attempt`/`finish_attempt` so the attempt count and
/// the retry/exhaustion decision stay consistent with the recorded status.
///
/// `db` is required: `TaskMaster` always records task status, so there is no
/// enqueue-only mode. Callers that don't want background-task bookkeeping
/// should simply not submit tasks.
///
/// Not Clone, share it via Arc.
pub struct TaskMaster {
    status: TaskStatusTracker,
    max_attempts: i32,
    illumination_queue: Option<Box<dyn TaskQueue<IlluminationTask>>>,
    search_index_queue: Option<Box<dyn TaskQueue<SearchIndexTask>>>,
    spark_queue: Option<Box<dyn TaskQueue<SparkTask>>>,
}

/// The outcome of a single task attempt, from the worker's point of view.
///
/// This tells the webhook handler which HTTP status to return so that Cloud
/// Tasks does (or doesn't) retry:
///
/// - `Completed` — 2xx. Task is done.
/// - `ErrorWillRetry` — non-2xx. Cloud Tasks retries within its budget.
/// - `ErrorExhausted` — **2xx**. The app has spent its own retry budget, so we
///   must ack the task to stop Cloud Tasks from spending its (larger) budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptOutcome {
    Completed,
    ErrorWillRetry,
    ErrorExhausted,
}

impl AttemptOutcome {
    /// Decide the outcome of a failed attempt.
    ///
    /// An attempt is worth retrying only when the error is transient *and* the
    /// app still has retry budget. `attempt` is 1-based.
    pub fn from_failure(err: &api::ApiError, attempt: i32, max_attempts: i32) -> Self {
        if err.is_retryable() && attempt < max_attempts {
            AttemptOutcome::ErrorWillRetry
        } else {
            AttemptOutcome::ErrorExhausted
        }
    }
}

/// The outcome of a submission, from the submitter's point of view.
///
/// A refusal is a *normal* outcome, not an error: submitting work that is
/// already in flight is expected (double-clicked upload, retried request), so
/// callers should not log it as a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// A new run was created and enqueued. `run` counts from 1.
    Enqueued { run: i32 },
    /// The latest run is still in flight, so nothing was enqueued. `run` is the
    /// run that is already in progress.
    RefusedInFlight { run: i32 },
}

/// Decide what to do about a submission given the latest run of the same
/// logical task.
///
/// - No prior run → start run 1.
/// - Latest run in flight → refuse; the work is already queued/running.
/// - Latest run settled → start the next run (this is what makes reruns work).
fn plan_submission(latest: Option<&model::task_status::Model>) -> Result<i32, i32> {
    let Some(latest) = latest else {
        return Ok(1);
    };

    match StatusCode::from_i32(latest.status_code) {
        Ok(status) if status.is_in_flight() => Err(latest.run),
        // An unreadable status is not something to build a refusal on: treat the
        // run as settled and let the unique index arbitrate if we're wrong.
        _ => Ok(latest.run + 1),
    }
}

/// Compute the 1-based attempt number for a run about to start.
///
/// Returns `None` when the run is already `Completed`: Cloud Tasks delivers at
/// least once, so a redelivery of finished work must not resurrect the row back
/// to `InProgress` (which would show a spurious "in progress" blip to any client
/// watching the task).
fn next_attempt_number(status: Option<&model::task_status::Model>) -> Option<i32> {
    match status {
        Some(row) if row.status_code == StatusCode::Completed.as_i32() => None,
        Some(row) => Some(row.attempts + 1),
        None => Some(1),
    }
}

impl TaskMaster {
    pub fn builder() -> TaskMasterBuilder {
        TaskMasterBuilder::default()
    }

    pub async fn submit_illumination(
        &self,
        user_id: i32,
        task: IlluminationTask,
    ) -> anyhow::Result<SubmitOutcome> {
        self.submit_inner(self.illumination_queue.as_ref(), user_id, task)
            .await
    }

    pub async fn submit_spark(
        &self,
        user_id: i32,
        task: SparkTask,
    ) -> anyhow::Result<SubmitOutcome> {
        if task.capture_ids.is_empty() {
            anyhow::bail!("submit_spark requires at least one capture_id");
        }
        self.submit_inner(self.spark_queue.as_ref(), user_id, task)
            .await
    }

    pub async fn submit_search_index(
        &self,
        user_id: i32,
        task: SearchIndexTask,
    ) -> anyhow::Result<SubmitOutcome> {
        self.submit_inner(self.search_index_queue.as_ref(), user_id, task)
            .await
    }

    /// Submit a task for execution.
    ///
    /// Refuses the submission if the latest run of this logical task is still in
    /// flight (see `plan_submission`), so a duplicate submit cannot queue the
    /// same work twice. If the latest run has settled, a **new run** is started,
    /// which is how reruns are expressed.
    async fn submit_inner<T: Task>(
        &self,
        queue: Option<&Box<dyn TaskQueue<T>>>,
        user_id: i32,
        task: T,
    ) -> anyhow::Result<SubmitOutcome> {
        let task_type = T::task_type();
        let envelope_id = TaskEnvelope::<T>::make_envelope_id(user_id, &task);

        let run = match plan_submission(self.status.latest_run(&envelope_id).await?.as_ref()) {
            Ok(run) => run,
            Err(run) => {
                tracing::debug!(
                    envelope_id = %envelope_id,
                    run,
                    "Refusing submit: the latest run is still in flight",
                );
                return Ok(SubmitOutcome::RefusedInFlight { run });
            }
        };

        let envelope = TaskEnvelope::new(user_id, task, run);

        let Some(queue) = queue else {
            tracing::warn!(
                envelope = ?envelope,
                "{} submitted but no queue configured, skipping enqueue.",
                task_type,
            );
            return Ok(SubmitOutcome::Enqueued { run });
        };

        // Record `Queued` before enqueueing, since `enqueue` moves the envelope.
        // `false` means another submit claimed this run first — refuse rather
        // than double-enqueue.
        let created = self
            .status
            .create_run(&envelope, StatusCode::Queued, 0)
            .await?;

        if !created {
            tracing::debug!(
                envelope_id = %envelope_id,
                run,
                "Refusing submit: lost a race for this run",
            );
            return Ok(SubmitOutcome::RefusedInFlight { run });
        }

        queue.enqueue(envelope.clone()).await.inspect_err(|err| {
            tracing::error!(
                queue = ?queue,
                envelope = ?envelope,
                error = ?err,
                "Failed to enqueue task",
            )
        })?;

        Ok(SubmitOutcome::Enqueued { run })
    }

    /// Mark an attempt as starting and return its 1-based attempt number.
    ///
    /// The attempt number is derived from the persisted `attempts` count, so it
    /// survives worker restarts and works identically for every queue backend
    /// (unlike Cloud Tasks' retry-count header, which the local queue lacks).
    ///
    /// Returns `None` if this run is already `Completed`. Cloud Tasks delivers
    /// at least once, so a redelivery of finished work must not resurrect it
    /// back to `InProgress` (which would show a spurious "in progress" blip to
    /// any client watching the task).
    pub async fn begin_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
    ) -> anyhow::Result<Option<i32>> {
        let status = self
            .status
            .query_run_status(&envelope.envelope_id, envelope.run)
            .await?;

        let Some(attempt) = next_attempt_number(status.as_ref()) else {
            tracing::info!(
                envelope_id = %envelope.envelope_id,
                run = envelope.run,
                "Ignoring attempt for already-completed run"
            );
            return Ok(None);
        };

        self.update_status(envelope, StatusCode::InProgress, attempt)
            .await?;

        Ok(Some(attempt))
    }

    /// Record the outcome of an attempt and decide whether Cloud Tasks should
    /// retry.
    ///
    /// A failed attempt is retryable only when the error is transient *and* the
    /// app still has retry budget; otherwise it is exhausted and must be acked.
    pub async fn finish_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        attempt: i32,
        result: &Result<(), api::ApiError>,
    ) -> anyhow::Result<AttemptOutcome> {
        match result {
            Ok(()) => {
                self.update_status(envelope, StatusCode::Completed, attempt)
                    .await?;
                Ok(AttemptOutcome::Completed)
            }
            Err(err) => {
                let outcome = AttemptOutcome::from_failure(err, attempt, self.max_attempts);
                let status = match outcome {
                    AttemptOutcome::ErrorWillRetry => StatusCode::ErrorWillRetry,
                    _ => StatusCode::ErrorExhausted,
                };

                self.update_status(envelope, status, attempt).await?;

                tracing::warn!(
                    envelope_id = %envelope.envelope_id,
                    attempt,
                    max_attempts = self.max_attempts,
                    retryable = err.is_retryable(),
                    status = %status,
                    error = ?err,
                    "Task attempt failed"
                );

                Ok(outcome)
            }
        }
    }

    /// Record a status transition for a run. `attempts` is the attempt count at
    /// the time of this transition.
    ///
    /// Private on purpose: external callers must use `begin_attempt`/`finish_attempt`,
    /// which keep `attempts` and the retry decision consistent with the status.
    async fn update_status<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.status.update_run(envelope, status, attempts).await
    }

    /// Query the *incomplete* task statuses recorded against a given entity,
    /// e.g. all tasks (`illuminate`, `search_index`, `spark`, ...) that
    /// operate on a single capture and have not yet succeeded. Always scoped by
    /// `user_id`.
    ///
    /// Includes `ErrorExhausted` — permanently failed work that the user still
    /// wants to see. Excludes `Completed`, which is vacuumed over time.
    pub async fn query_incomplete_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        self.status
            .query_incomplete_for_entity(user_id, entity_type, entity_id)
            .await
    }

    /// Query every *incomplete* task status for a user, across all entities.
    ///
    /// The user-level counterpart to `query_incomplete_for_entity`, for callers
    /// that want every outstanding task a user has rather than the tasks for
    /// one entity. Includes `ErrorExhausted`; excludes `Completed`.
    pub async fn query_incomplete_for_user(
        &self,
        user_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        self.status.query_incomplete_for_user(user_id).await
    }
}

#[derive(Default)]
pub struct TaskMasterBuilder {
    db: Option<DbHandle>,
    max_attempts: Option<i32>,
    illumination_queue: Option<Box<dyn TaskQueue<IlluminationTask>>>,
    search_index_queue: Option<Box<dyn TaskQueue<SearchIndexTask>>>,
    spark_queue: Option<Box<dyn TaskQueue<SparkTask>>>,
}

impl TaskMasterBuilder {
    pub fn db(mut self, db: DbHandle) -> Self {
        self.db = Some(db);
        self
    }
    /// Maximum number of attempts per task. See `Config::task_max_attempts`.
    pub fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }

    pub fn illumination_queue(
        mut self,
        illumination_queue: impl TaskQueue<IlluminationTask> + 'static,
    ) -> Self {
        self.illumination_queue = Some(Box::new(illumination_queue));
        self
    }

    pub fn search_index_queue(
        mut self,
        search_index_queue: impl TaskQueue<SearchIndexTask> + 'static,
    ) -> Self {
        self.search_index_queue = Some(Box::new(search_index_queue));
        self
    }

    pub fn spark_queue(mut self, spark_queue: impl TaskQueue<SparkTask> + 'static) -> Self {
        self.spark_queue = Some(Box::new(spark_queue));
        self
    }

    pub fn build(self) -> anyhow::Result<TaskMaster> {
        let Some(db) = self.db else {
            anyhow::bail!("TaskMaster requires a database handle");
        };

        Ok(TaskMaster {
            status: TaskStatusTracker::new(db),
            // Mirrors `Config::task_max_attempts`'s default so a builder that
            // forgets `.max_attempts(..)` behaves like production rather than
            // silently disabling retries.
            max_attempts: self.max_attempts.unwrap_or(3).max(1),
            illumination_queue: self.illumination_queue,
            search_index_queue: self.search_index_queue,
            spark_queue: self.spark_queue,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn transient_failure_retries_until_budget_is_spent() {
        let err = api::ApiError::internal(anyhow::anyhow!("upstream unavailable"));
        let max = 3;

        assert_eq!(
            AttemptOutcome::from_failure(&err, 1, max),
            AttemptOutcome::ErrorWillRetry
        );
        assert_eq!(
            AttemptOutcome::from_failure(&err, 2, max),
            AttemptOutcome::ErrorWillRetry
        );
        // Last permitted attempt exhausts rather than retrying forever.
        assert_eq!(
            AttemptOutcome::from_failure(&err, 3, max),
            AttemptOutcome::ErrorExhausted
        );
        assert_eq!(
            AttemptOutcome::from_failure(&err, 4, max),
            AttemptOutcome::ErrorExhausted
        );
    }

    #[test]
    fn permanent_failure_exhausts_immediately() {
        let err = api::ApiError::bad_request(anyhow::anyhow!("capture_ids must be non-empty"));
        let max = 3;

        // Even with budget remaining, a non-retryable error stops on attempt 1.
        assert_eq!(
            AttemptOutcome::from_failure(&err, 1, max),
            AttemptOutcome::ErrorExhausted
        );
    }

    #[test]
    fn max_attempts_of_one_never_retries() {
        let err = api::ApiError::internal(anyhow::anyhow!("upstream unavailable"));

        assert_eq!(
            AttemptOutcome::from_failure(&err, 1, 1),
            AttemptOutcome::ErrorExhausted
        );
    }

    #[test]
    fn first_attempt_of_unknown_task_is_one() {
        assert_eq!(next_attempt_number(None), Some(1));
    }

    #[test]
    fn attempt_number_increments_from_persisted_count() {
        let row = status_row(StatusCode::ErrorWillRetry, 2);

        assert_eq!(next_attempt_number(Some(&row)), Some(3));
    }

    #[test]
    fn completed_task_is_not_resurrected() {
        let row = status_row(StatusCode::Completed, 1);

        assert_eq!(
            next_attempt_number(Some(&row)),
            None,
            "an at-least-once redelivery of finished work must be ignored"
        );
    }

    #[test]
    fn submission_starts_at_run_one_when_never_run() {
        assert_eq!(plan_submission(None), Ok(1));
    }

    #[test]
    fn submission_is_refused_while_a_run_is_in_flight() {
        for status in [
            StatusCode::Queued,
            StatusCode::InProgress,
            StatusCode::ErrorWillRetry,
        ] {
            let row = status_row(status, 1);

            assert_eq!(
                plan_submission(Some(&row)),
                Err(1),
                "{status} means a worker may still run, so the submit is refused"
            );
        }
    }

    #[test]
    fn submission_reruns_after_a_settled_run() {
        for status in [StatusCode::Completed, StatusCode::ErrorExhausted] {
            let row = status_row_of_run(status, 1, 3);

            assert_eq!(
                plan_submission(Some(&row)),
                Ok(4),
                "{status} is settled, so the next run is permitted"
            );
        }
    }

    /// A `task_status` row with only the fields the pure helpers read set to
    /// meaningful values.
    fn status_row(status: StatusCode, attempts: i32) -> model::task_status::Model {
        status_row_of_run(status, attempts, 1)
    }

    fn status_row_of_run(status: StatusCode, attempts: i32, run: i32) -> model::task_status::Model {
        model::task_status::Model {
            id: 0,
            user_id: 1,
            envelope_id: "u1-illuminate-capture1".to_string(),
            run,
            task_type: "illuminate".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 1,
            status_code: status.as_i32(),
            attempts,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[derive(Debug, Clone)]
    struct RecordingQueue {
        captures: Arc<Mutex<Vec<i32>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl TaskQueue<IlluminationTask> for RecordingQueue {
        async fn enqueue(&self, envelope: TaskEnvelope<IlluminationTask>) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("enqueue failed")
            }

            let mut captures = self
                .captures
                .lock()
                .expect("RecordingQueue captures mutex should not be poisoned");
            captures.push(envelope.task.unwrap().capture_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn submit_illumination_enqueues_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let captures = Arc::new(Mutex::new(Vec::new()));
        let queue = RecordingQueue {
            captures: Arc::clone(&captures),
            fail: false,
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(queue)
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let recorded = captures
            .lock()
            .expect("captures mutex should not be poisoned")
            .clone();
        assert_eq!(recorded, vec![42]);
    }

    #[tokio::test]
    async fn submit_without_queue_is_noop() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illumination(1, IlluminationTask { capture_id: 7 })
            .await
            .expect("submit should be a no-op when queue is absent");
    }

    #[tokio::test]
    async fn submit_propagates_enqueue_error() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let queue = RecordingQueue {
            captures: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        };
        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(queue)
            .build()
            .expect("build should succeed with a db");

        let result = service
            .submit_illumination(1, IlluminationTask { capture_id: 9 })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_without_db_is_an_error() {
        let result = TaskMaster::builder().build();
        assert!(result.is_err(), "TaskMaster must require a database");
    }

    /// A submitted task is recorded as `Queued` with zero attempts.
    #[tokio::test]
    async fn submit_records_queued_row() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let rows = service
            .query_incomplete_for_entity(1, "capture", 42)
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, StatusCode::Queued.as_i32());
        assert_eq!(rows[0].attempts, 0);
        assert_eq!(rows[0].envelope_id, "u1-illuminate-capture42");
        assert_eq!(rows[0].run, 1);
    }

    /// A duplicate submit while a run is in flight is refused, not enqueued.
    #[tokio::test]
    async fn duplicate_submit_while_in_flight_is_refused() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let captures = Arc::new(Mutex::new(Vec::new()));
        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::clone(&captures),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let first = service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");
        let second = service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("second submit should be answered, not error");

        assert_eq!(first, SubmitOutcome::Enqueued { run: 1 });
        assert_eq!(second, SubmitOutcome::RefusedInFlight { run: 1 });
        assert_eq!(
            captures.lock().unwrap().len(),
            1,
            "the duplicate must not reach the queue"
        );
    }

    /// Once a run settles, a new submit starts a *new run* rather than being
    /// refused — this is how reruns are expressed.
    #[tokio::test]
    async fn submit_after_completion_starts_a_new_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");

        // Complete run 1.
        let envelope = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 1);
        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("first attempt should not be skipped");
        service
            .finish_attempt(&envelope, attempt, &Ok(()))
            .await
            .expect("finish_attempt should succeed");

        let rerun = service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("rerun submit should succeed");

        assert_eq!(rerun, SubmitOutcome::Enqueued { run: 2 });
    }

    /// Runs 1 and 2 both exist; only the latest is reported as incomplete, so a
    /// rerun supersedes rather than duplicating the logical task.
    #[tokio::test]
    async fn incomplete_query_returns_only_the_latest_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .max_attempts(1) // exhaust on the first failure
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1 exhausts (incomplete, settled).
        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");
        let envelope = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 1);
        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("first attempt should not be skipped");
        let outcome = service
            .finish_attempt(
                &envelope,
                attempt,
                &Err(api::ApiError::internal(anyhow::anyhow!("boom"))),
            )
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(
            outcome,
            AttemptOutcome::ErrorExhausted,
            "max_attempts=1 means the first failure is terminal"
        );

        // Run 2 is queued (incomplete, in flight).
        let rerun = service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("rerun submit should succeed");
        assert_eq!(rerun, SubmitOutcome::Enqueued { run: 2 });

        let rows = service
            .query_incomplete_for_entity(1, "capture", 42)
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1, "only the latest run is reported");
        assert_eq!(rows[0].run, 2);
        assert_eq!(rows[0].status_code, StatusCode::Queued.as_i32());
    }

    /// A stale incomplete run must not shadow a newer completed one: the
    /// logical task is done, so it must not appear as outstanding.
    #[tokio::test]
    async fn completed_latest_run_hides_an_older_failed_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .max_attempts(1) // exhaust on the first failure
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1 exhausts and stays incomplete.
        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");
        let first = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 1);
        let attempt = service
            .begin_attempt(&first)
            .await
            .expect("begin_attempt should succeed")
            .expect("first attempt should not be skipped");
        service
            .finish_attempt(
                &first,
                attempt,
                &Err(api::ApiError::internal(anyhow::anyhow!("boom"))),
            )
            .await
            .expect("finish_attempt should succeed");

        // Run 2 completes.
        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("rerun submit should succeed");
        let second = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 2);
        let attempt = service
            .begin_attempt(&second)
            .await
            .expect("begin_attempt should succeed")
            .expect("second run attempt should not be skipped");
        service
            .finish_attempt(&second, attempt, &Ok(()))
            .await
            .expect("finish_attempt should succeed");

        let rows = service
            .query_incomplete_for_entity(1, "capture", 42)
            .await
            .expect("query should succeed");

        assert!(
            rows.is_empty(),
            "the latest run completed, so nothing is outstanding"
        );
    }

    /// Attempt counting is per-run: a rerun starts its attempts from scratch.
    #[tokio::test]
    async fn attempts_are_scoped_to_a_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .max_attempts(2) // two attempts, then exhaust
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1: two attempts, then exhaust.
        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");
        let first = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 1);
        let failure = Err(api::ApiError::internal(anyhow::anyhow!("boom")));

        let attempt = service
            .begin_attempt(&first)
            .await
            .expect("begin_attempt should succeed")
            .expect("first attempt should not be skipped");
        assert_eq!(attempt, 1);
        let outcome = service
            .finish_attempt(&first, attempt, &failure)
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(outcome, AttemptOutcome::ErrorWillRetry);

        let attempt = service
            .begin_attempt(&first)
            .await
            .expect("begin_attempt should succeed")
            .expect("second attempt should not be skipped");
        assert_eq!(attempt, 2);
        let outcome = service
            .finish_attempt(&first, attempt, &failure)
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(
            outcome,
            AttemptOutcome::ErrorExhausted,
            "run 1 has now spent its budget"
        );

        // Run 2 begins at attempt 1 again, even though run 1 used attempts.
        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("rerun submit should succeed");
        let second = TaskEnvelope::new(1, IlluminationTask { capture_id: 42 }, 2);
        let attempt = service
            .begin_attempt(&second)
            .await
            .expect("begin_attempt should succeed")
            .expect("second run attempt should not be skipped");

        assert_eq!(attempt, 1, "each run counts its own attempts");
    }

    /// The full attempt lifecycle: `Queued` -> `InProgress` -> `Completed`.
    #[tokio::test]
    async fn attempt_lifecycle_reaches_completed() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 7 };
        service
            .submit_illumination(1, task.clone())
            .await
            .expect("submit should succeed");

        let envelope = TaskEnvelope::new(1, task, 1);

        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("first attempt should not be skipped");
        assert_eq!(attempt, 1);

        let outcome = service
            .finish_attempt(&envelope, attempt, &Ok(()))
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(outcome, AttemptOutcome::Completed);

        // Completed rows are excluded from the incomplete query.
        let rows = service
            .query_incomplete_for_entity(1, "capture", 7)
            .await
            .expect("query should succeed");
        assert!(rows.is_empty(), "completed work is not incomplete");
    }

    /// A transient failure retries while budget remains, then exhausts.
    #[tokio::test]
    async fn transient_failure_escalates_to_exhausted() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .max_attempts(2)
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 9 };
        service
            .submit_illumination(1, task.clone())
            .await
            .expect("submit should succeed");

        let envelope = TaskEnvelope::new(1, task, 1);
        let failure = Err(api::ApiError::internal(anyhow::anyhow!("transient")));

        // Attempt 1: budget remains, so it will retry.
        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");
        let outcome = service
            .finish_attempt(&envelope, attempt, &failure)
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(outcome, AttemptOutcome::ErrorWillRetry);

        // Attempt 2: budget spent, so it exhausts.
        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");
        assert_eq!(attempt, 2);
        let outcome = service
            .finish_attempt(&envelope, attempt, &failure)
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(outcome, AttemptOutcome::ErrorExhausted);

        // Exhausted work is still incomplete (the user should see it).
        let rows = service
            .query_incomplete_for_entity(1, "capture", 9)
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, StatusCode::ErrorExhausted.as_i32());
        assert_eq!(rows[0].attempts, 2);
    }

    /// A redelivery of already-completed work is not resurrected.
    #[tokio::test]
    async fn completed_task_is_not_resurrected_in_db() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 11 };
        service
            .submit_illumination(1, task.clone())
            .await
            .expect("submit should succeed");

        let envelope = TaskEnvelope::new(1, task, 1);

        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");
        service
            .finish_attempt(&envelope, attempt, &Ok(()))
            .await
            .expect("finish_attempt should succeed");

        // A redelivery must be skipped, not flipped back to InProgress.
        let redelivery = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed");
        assert!(
            redelivery.is_none(),
            "completed work must not be resurrected"
        );
    }

    /// The user-scoped query returns incomplete work across entities.
    #[tokio::test]
    async fn query_incomplete_for_user_spans_entities() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        for capture_id in [1, 2, 3] {
            service
                .submit_illumination(1, IlluminationTask { capture_id })
                .await
                .expect("submit should succeed");
        }

        let rows = service
            .query_incomplete_for_user(1)
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 3);

        // Another user sees nothing.
        let other = service
            .query_incomplete_for_user(2)
            .await
            .expect("query should succeed");
        assert!(other.is_empty(), "queries must be scoped by user_id");
    }
}
