use crate::api;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::model;

use super::*;

/// The primary entry point for manipulating `Task` instances.
///
/// Owns the backend queues **and** the `task_status` table. Everything else
/// talks to tasks through this API:
///
/// - `submit_*` — enqueue + record a `Queued` row.
/// - `begin_attempt` / `finish_attempt` — the worker-side attempt lifecycle.
/// - `query_*` — read status (replay / polling).
///
/// Status is deliberately not exposed as a raw setter: workers go through
/// `begin_attempt`/`finish_attempt` so `attempts` and the retry decision stay
/// consistent with the recorded status.
///
/// Not `Clone`; share via `Arc`.
pub struct TaskMaster {
    status: TaskStatusTracker,
    max_attempts_per_run: i32, // mirrors `Config::task_max_attempts`
    illumination_queue: Option<Box<dyn TaskQueue<IlluminationTask>>>,
    search_index_queue: Option<Box<dyn TaskQueue<SearchIndexTask>>>,
    spark_queue: Option<Box<dyn TaskQueue<SparkTask>>>,
}

/// The outcome of a Task submission.
///
/// We refuse duplicate submissions of work if the previous run is still in
/// flight, this is expressed as `RefusedAlreadyInFlight` rather than an
/// error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// A new run was created and enqueued. `run` counts from 1.
    Enqueued { run: i32 },
    /// The latest run is still in flight, so nothing was enqueued. `run` is the
    /// in-flight run that blocked the submission.
    RefusedAlreadyInFlight { run: i32 },
}

// Determine whether a new run is permitted, and if so what its 1-based run number is.
fn decide_next_run(latest_run: Option<&model::task_status::Model>) -> Option<i32> {
    if let Some(latest) = latest_run {
        match StatusCode::from_i32(latest.status_code) {
            // If latest run is still in flight, refuse the submission
            Ok(status) if status.is_in_flight() => None,
            // Latest run is not in flight, so a rerun is permitted
            _ => Some(latest.run + 1),
        }
    } else {
        return Some(1); // no prior run, so this is the first
    }
}

/// Determine the 1-based attempt number for a run about to start.
///
/// Cloud Tasks delivers at least once, so a redelivery of finished work must
/// not resurrect it to `InProgress`.
fn decide_next_attempt(status: Option<&model::task_status::Model>) -> Option<i32> {
    match status {
        Some(row) if row.status_code == StatusCode::CompleteSuccess.as_i32() => None,
        Some(row) => Some(row.attempts + 1),
        None => Some(1),
    }
}

fn decide_will_retry(err: &api::ApiError, attempt: i32, max_attempts: i32) -> bool {
    attempt < max_attempts && err.is_retryable()
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
        self.submit_inner(self.illumination_queue.as_deref(), user_id, task)
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
        self.submit_inner(self.spark_queue.as_deref(), user_id, task)
            .await
    }

    pub async fn submit_search_index(
        &self,
        user_id: i32,
        task: SearchIndexTask,
    ) -> anyhow::Result<SubmitOutcome> {
        self.submit_inner(self.search_index_queue.as_deref(), user_id, task)
            .await
    }

    /// Submit a task for execution.
    ///
    /// Wraps the Task in a TaskEnvelope, enqueues it in the corresponding backend,
    /// and records a `Queued` row in the `task_status` table for that envelope_id.
    ///
    /// Tasks submitted for the first time start a "run" of 1.
    ///
    /// If there are already existing run(s) for the same envelope_id, then behavior
    /// depends on the status of that most recent run:
    ///   - If the latest run is still in flight (Queued, InProgress, or
    ///     ErrorWillRetry), then submission fails (to avoid duplicate work) and returns
    ///     `SubmitOutcome::RefusedAlreadyInFlight`.
    ///   - If the latest run is complete (**either** CompleteSuccess **or**
    ///     CompleteFailure), then submission starts a **new run** (run number
    ///     incremented by 1) and returns `SubmitOutcome::Enqueued`. This is how
    ///     intentional reruns of the same logical task are expressed.
    async fn submit_inner<T: Task>(
        &self,
        queue: Option<&dyn TaskQueue<T>>, // TODO SHOULD NOT BE OPTIONAL
        user_id: i32,
        task: T,
    ) -> anyhow::Result<SubmitOutcome> {
        let task_type = T::task_type();
        let envelope_id = TaskEnvelope::<T>::make_envelope_id(user_id, &task);

        let latest_run = self.status.latest_run(&envelope_id).await?;
        let next_run = match decide_next_run(latest_run.as_ref()) {
            Some(run) => run,
            None => {
                // `decide_next_run` refuses only when a run already exists.
                let run = latest_run
                    .as_ref()
                    .expect("refusal implies an existing run")
                    .run;
                tracing::debug!(
                    envelope_id = %envelope_id,
                    run,
                    "Refusing submit: latest run is still in flight",
                );
                return Ok(SubmitOutcome::RefusedAlreadyInFlight { run });
            }
        };

        let envelope = TaskEnvelope::new(user_id, task, next_run);

        let Some(queue) = queue else {
            tracing::warn!(
                envelope = ?envelope,
                "{} submitted but no queue configured, skipping enqueue.",
                task_type,
            );
            return Ok(SubmitOutcome::Enqueued { run: next_run }); // THIS IS WRONG
        };

        // Record `Queued` before enqueueing, since `enqueue` moves the envelope.
        // `false` = another submit claimed this run first; refuse, don't double-enqueue.
        // TODO should we rethink this and possibly record QueueFailure as a status code?
        let created = self
            .status
            .create_run(&envelope, StatusCode::Queued, 0)
            .await?;

        if !created {
            tracing::debug!(
                envelope_id = %envelope_id,
                next_run,
                "Refusing submit: lost a race for this run",
            );
            return Ok(SubmitOutcome::RefusedAlreadyInFlight { run: next_run });
        }

        queue.enqueue(envelope.clone()).await.inspect_err(|err| {
            tracing::error!(
                queue = ?queue,
                envelope = ?envelope,
                error = ?err,
                "Failed to enqueue task",
            )
        })?;

        Ok(SubmitOutcome::Enqueued { run: next_run })
    }

    /// Mark an attempt as starting and return its 1-based attempt number.
    ///
    /// Derived from the persisted `attempts` count, so it survives worker
    /// restarts and works for every queue backend (Cloud Tasks' retry-count
    /// header isn't available on the local queue). `None` if the run already
    /// `CompleteSuccess` — that redelivery must be ignored, not resurrected.
    pub async fn begin_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
    ) -> anyhow::Result<Option<i32>> {
        let status = self
            .status
            .query_run_status(&envelope.envelope_id, envelope.run)
            .await?;

        let Some(attempt) = decide_next_attempt(status.as_ref()) else {
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
    /// retry (transient error *and* budget remaining; otherwise exhausted).
    pub async fn finish_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        attempt: i32,
        run_result: &Result<(), api::ApiError>,
    ) -> anyhow::Result<StatusCode> {
        match run_result {
            Ok(()) => {
                self.update_status(envelope, StatusCode::CompleteSuccess, attempt)
                    .await?;
                Ok(StatusCode::CompleteSuccess)
            }
            Err(err) => {
                let status_code = if decide_will_retry(err, attempt, self.max_attempts_per_run) {
                    StatusCode::ErrorWillRetry
                } else {
                    StatusCode::CompleteFailure
                };

                self.update_status(envelope, status_code, attempt).await?;

                tracing::warn!(
                    envelope_id = %envelope.envelope_id,
                    attempt,
                    max_attempts = self.max_attempts_per_run,
                    retryable = err.is_retryable(),
                    status_code = %status_code,
                    error = ?err,
                    "Task attempt failed"
                );

                Ok(status_code)
            }
        }
    }

    /// Record a status transition for a run. Private: callers must use
    /// `begin_attempt`/`finish_attempt`.
    async fn update_status<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.status.update_run(envelope, status, attempts).await
    }

    /// Incomplete task statuses recorded against one entity, scoped by
    /// `user_id`. Includes `ErrorExhausted` (the user still wants to see
    /// failed work); excludes `Completed`.
    pub async fn query_incomplete_for_entity(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        self.status
            .query_task_status_for_entity(user_id, entity_type, entity_id)
            .await
    }

    /// Every incomplete task status for a user, across all entities. The
    /// user-level counterpart to `query_incomplete_for_entity`.
    pub async fn query_incomplete_for_user(
        &self,
        user_id: i32,
    ) -> anyhow::Result<Vec<model::task_status::Model>> {
        self.status.query_task_status_for_user(user_id).await
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
            // Mirrors `Config::task_max_attempts` so a builder that forgets
            // `.max_attempts(..)` behaves like production rather than disabling retries.
            max_attempts_per_run: self.max_attempts.unwrap_or(3).max(1),
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

        assert!(decide_will_retry(&err, 1, max));
        assert!(decide_will_retry(&err, 2, max));
        // Last permitted attempt exhausts rather than retrying forever.
        assert!(!decide_will_retry(&err, 3, max));
        assert!(!decide_will_retry(&err, 4, max));
    }

    #[test]
    fn permanent_failure_exhausts_immediately() {
        let err = api::ApiError::bad_request(anyhow::anyhow!("capture_ids must be non-empty"));
        let max = 3;

        // Even with budget remaining, a non-retryable error stops on attempt 1.
        assert!(!decide_will_retry(&err, 1, max));
    }

    #[test]
    fn max_attempts_of_one_never_retries() {
        let err = api::ApiError::internal(anyhow::anyhow!("upstream unavailable"));

        assert!(!decide_will_retry(&err, 1, 1));
    }

    #[test]
    fn first_attempt_of_unknown_task_is_one() {
        assert_eq!(decide_next_attempt(None), Some(1));
    }

    #[test]
    fn attempt_number_increments_from_persisted_count() {
        let row = status_row(StatusCode::ErrorWillRetry, 2);

        assert_eq!(decide_next_attempt(Some(&row)), Some(3));
    }

    #[test]
    fn completed_task_is_not_resurrected() {
        let row = status_row(StatusCode::CompleteSuccess, 1);

        assert_eq!(
            decide_next_attempt(Some(&row)),
            None,
            "an at-least-once redelivery of finished work must be ignored"
        );
    }

    #[test]
    fn submission_starts_at_run_one_when_never_run() {
        assert_eq!(decide_next_run(None), Some(1));
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
                decide_next_run(Some(&row)),
                None,
                "{status} means a worker may still run, so the submit is refused"
            );
        }
    }

    #[test]
    fn submission_reruns_after_a_settled_run() {
        for status in [StatusCode::CompleteSuccess, StatusCode::CompleteFailure] {
            let row = status_row_of_run(status, 1, 3);

            assert_eq!(
                decide_next_run(Some(&row)),
                Some(4),
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
        assert_eq!(second, SubmitOutcome::RefusedAlreadyInFlight { run: 1 });
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
            StatusCode::CompleteFailure,
            "max_attempts=1 means the first failure is terminal"
        );

        // Run 2 is queued (in flight).
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

    /// A rerun supersedes the run before it: only the latest run is reported,
    /// even when the older run failed and the newer one completed.
    #[tokio::test]
    async fn completed_latest_run_supersedes_an_older_failed_run() {
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

        assert_eq!(rows.len(), 1, "only the latest run is reported");
        assert_eq!(rows[0].run, 2, "the completed rerun supersedes run 1");
        assert_eq!(rows[0].status_code, StatusCode::CompleteSuccess.as_i32());
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
        assert_eq!(outcome, StatusCode::ErrorWillRetry);

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
            StatusCode::CompleteFailure,
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

    /// The full attempt lifecycle: `Queued` -> `InProgress` -> `CompleteSuccess`.
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
        assert_eq!(outcome, StatusCode::CompleteSuccess);

        // The completed row is reported: a caller must be able to see that its
        // work finished.
        let rows = service
            .query_incomplete_for_entity(1, "capture", 7)
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, StatusCode::CompleteSuccess.as_i32());
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
        assert_eq!(outcome, StatusCode::ErrorWillRetry);

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
        assert_eq!(outcome, StatusCode::CompleteFailure);

        // The exhausted run is still reported (the user should see it).
        let rows = service
            .query_incomplete_for_entity(1, "capture", 9)
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, StatusCode::CompleteFailure.as_i32());
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

    /// The user-scoped query returns work across entities.
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

    /// A service with an illumination queue that records (or fails) enqueues.
    fn service(db: &crate::test_support::test_db::TestDb, queue_fails: bool) -> TaskMaster {
        TaskMaster::builder()
            .db(db.handle())
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: queue_fails,
            })
            .build()
            .expect("build should succeed with a db")
    }

    #[tokio::test]
    async fn submit_spark_requires_at_least_one_capture() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = service(&db, false);

        let result = service
            .submit_spark(
                1,
                SparkTask {
                    spark_id: 1,
                    capture_ids: vec![],
                },
            )
            .await;

        assert!(
            result.is_err(),
            "an empty spark has no meaning and must be rejected"
        );
    }

    /// With no queue configured the submit is accepted but nothing is recorded,
    /// and it is NOT enqueued anywhere. This is the util/test path.
    #[tokio::test]
    async fn submit_without_queue_records_no_row() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = TaskMaster::builder()
            .db(db.handle())
            .build()
            .expect("build should succeed with a db");

        let outcome = service
            .submit_illumination(1, IlluminationTask { capture_id: 7 })
            .await
            .expect("submit should be answered, not error");

        assert_eq!(outcome, SubmitOutcome::Enqueued { run: 1 });

        let rows = service
            .query_incomplete_for_user(1)
            .await
            .expect("query should succeed");
        assert!(
            rows.is_empty(),
            "without a queue there is no work to track, so no row is written"
        );
    }

    /// The row is written *before* the enqueue, so a failed enqueue leaves a
    /// `Queued` row that nothing will ever pick up. Documented as a tolerated
    /// orphan (see pragmatism.md) — this test pins the behaviour so a change is
    /// deliberate rather than accidental.
    #[tokio::test]
    async fn failed_enqueue_leaves_a_queued_row() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = service(&db, true);

        let result = service
            .submit_illumination(1, IlluminationTask { capture_id: 9 })
            .await;
        assert!(result.is_err(), "the enqueue error must propagate");

        let rows = service
            .query_incomplete_for_entity(1, "capture", 9)
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, StatusCode::Queued.as_i32());
    }

    /// Only `Completed` is protected from redelivery. An exhausted run is not,
    /// so a redelivery would move it back to `InProgress`.
    ///
    /// Unreachable today: we ack `ErrorExhausted` with a 2xx, so Cloud Tasks
    /// stops redelivering. Pinned here because the boundary is subtle and would
    /// matter if the ack policy ever changed.
    #[tokio::test]
    async fn exhausted_run_is_not_protected_from_redelivery() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = TaskMaster::builder()
            .db(db.handle())
            .max_attempts(1)
            .illumination_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illumination(1, IlluminationTask { capture_id: 5 })
            .await
            .expect("submit should succeed");

        let envelope = TaskEnvelope::new(1, IlluminationTask { capture_id: 5 }, 1);
        let attempt = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");
        let outcome = service
            .finish_attempt(
                &envelope,
                attempt,
                &Err(api::ApiError::internal(anyhow::anyhow!("boom"))),
            )
            .await
            .expect("finish_attempt should succeed");
        assert_eq!(outcome, StatusCode::CompleteFailure);

        let redelivery = service
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed");

        assert_eq!(
            redelivery,
            Some(2),
            "an exhausted run is retried if redelivered, not skipped"
        );
    }

    /// Two logical tasks for the same entity coexist: the status queries key on
    /// the envelope, and a second task type must not overwrite or hide the
    /// first.
    #[tokio::test]
    async fn distinct_task_types_for_one_entity_are_independent() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = service(&db, false);

        service
            .submit_illumination(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let rows = service
            .query_incomplete_for_entity(1, "capture", 42)
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].envelope_id, "u1-illuminate-capture42",
            "the envelope carries the task type, so another task type cannot collide"
        );
    }

    /// `attempts` comes from the DB, not memory, so a worker restart resumes the
    /// count instead of resetting it.
    #[tokio::test]
    async fn attempt_count_survives_a_new_task_master() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        // First "process": one attempt that will retry.
        let first_process = service(&db, false);
        first_process
            .submit_illumination(1, IlluminationTask { capture_id: 3 })
            .await
            .expect("submit should succeed");
        let envelope = TaskEnvelope::new(1, IlluminationTask { capture_id: 3 }, 1);
        let attempt = first_process
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");
        first_process
            .finish_attempt(
                &envelope,
                attempt,
                &Err(api::ApiError::internal(anyhow::anyhow!("transient"))),
            )
            .await
            .expect("finish_attempt should succeed");

        // Second "process" over the same database: the count is not reset.
        let second_process = service(&db, false);
        let attempt = second_process
            .begin_attempt(&envelope)
            .await
            .expect("begin_attempt should succeed")
            .expect("attempt should not be skipped");

        assert_eq!(attempt, 2, "the attempt count is persisted, not in-memory");
    }
}
