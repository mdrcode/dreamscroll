use std::sync::Arc;

use crate::api;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::model;
use crate::sse::ServerEventNotifier;

use super::taskruntracker::TaskRunTracker;
use super::*;

/// The primary entry point for manipulating `Task` instances.
///
/// Owns the backend queues **and** the TaskRunTracker. Everything else
/// talks to tasks through this API:
///
/// - `submit_*` — enqueue in the backend, and record a `Queued` row.
/// - `begin_attempt` / `finish_attempt` — manage lifecycle of a Task Run
/// - `query_*` — read status (replay / polling).
///
/// Status is deliberately not exposed as a raw setter: workers go through
/// `begin_attempt`/`finish_attempt` so `attempts` and the retry decision stay
/// consistent with the recorded status.
///
/// Not `Clone`; share via `Arc`.
pub struct TaskMaster {
    run_tracker: TaskRunTracker,
    max_attempts_per_run: i32, // mirrors `Config::task_max_attempts`
    illuminate_queue: Box<dyn TaskQueue<IlluminationTask>>,
    search_index_queue: Box<dyn TaskQueue<SearchIndexTask>>,
    spark_queue: Box<dyn TaskQueue<SparkTask>>,
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
    /// The latest run is still in flight, so nothing was enqueued.
    RefusedAlreadyInFlight,
}

/// Return the next 1-based run number, or `None` if the latest run is in flight.
fn decide_next_run(latest_run: Option<&model::task_run_status::Model>) -> Option<i32> {
    if let Some(latest) = latest_run {
        match TaskRunStatus::from_i32(latest.status_code) {
            Ok(status) if status.is_in_flight() => None,
            _ => Some(latest.run + 1),
        }
    } else {
        Some(1)
    }
}

/// Determine the 1-based attempt number for a run about to start.
///
/// Cloud Tasks delivers at least once, so a redelivery of finished work must
/// not resurrect it to `InProgress`.
fn decide_next_attempt(status: Option<&model::task_run_status::Model>) -> Option<i32> {
    match status {
        Some(row) if row.status_code == TaskRunStatus::CompleteSuccess.as_i32() => None,
        Some(row) if row.status_code == TaskRunStatus::SubmissionFailed.as_i32() => None,
        Some(row) => Some(row.attempts + 1),
        None => Some(1),
    }
}

/// Determines whether a failed attempt should be retried.
///
/// We define internal retry-ability to the ApiError code
fn decide_will_retry(err: &api::ApiError, attempt: i32, max_attempts: i32) -> bool {
    attempt < max_attempts && err.is_retryable()
}

impl TaskMaster {
    pub fn builder() -> TaskMasterBuilder {
        TaskMasterBuilder::default()
    }

    pub async fn submit_illuminate(
        &self,
        user_id: i32,
        task: IlluminationTask,
    ) -> anyhow::Result<SubmitOutcome> {
        self.submit_inner(self.illuminate_queue.as_ref(), user_id, task)
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
    /// Wraps the Task in a TaskEnvelope, enqueues it in the corresponding backend,
    /// and records a `Queued` row in the `task_run_status` table for that envelope_id.
    /// If enqueueing fails, the row is changed to `SubmissionFailed` before the
    /// enqueue error is returned.
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
        queue: &dyn TaskQueue<T>,
        user_id: i32,
        task: T,
    ) -> anyhow::Result<SubmitOutcome> {
        let envelope_id = TaskEnvelope::<T>::make_envelope_id(user_id, &task);

        let latest = self.run_tracker.query_latest_run(&envelope_id).await?;
        let next_run = match decide_next_run(latest.as_ref()) {
            Some(run) => run,
            None => {
                tracing::debug!(
                    envelope_id = %envelope_id,
                    "Refused submit: latest run still in flight",
                );
                return Ok(SubmitOutcome::RefusedAlreadyInFlight);
            }
        };

        let envelope = TaskEnvelope::new(user_id, task, next_run);

        // Create the Run and record `Queued` (in the database) before truly
        // enqueueing (in the backend).
        // `false` = another submit won the raise and claimed this run first
        if !self.run_tracker.create_run(&envelope).await? {
            tracing::debug!(
                envelope_id = %envelope_id,
                next_run,
                "Refusing submit: lost the race",
            );
            return Ok(SubmitOutcome::RefusedAlreadyInFlight);
        }

        // Tracker publishes Queued in the DB before enqueue in the backend,
        // so that a fast worker cannot publish InProgress first and then be
        // followed by this stale hint. Now, we actually try to enqueue in the
        // backend (theoretically execution could start immediately).
        if let Err(enqueue_err) = queue.enqueue(envelope.clone()).await {
            tracing::error!(
                queue = ?queue,
                envelope = ?envelope,
                error = ?enqueue_err,
                "Failed to enqueue task",
            );

            self.run_tracker
                .update_run(&envelope, TaskRunStatus::SubmissionFailed, 0)
                .await?;

            return Err(enqueue_err);
        }

        Ok(SubmitOutcome::Enqueued { run: next_run })
    }

    /// Mark an attempt as starting and return its 1-based attempt number.
    ///
    /// Derived from the persisted `attempts` count, so it survives worker
    /// restarts and works for every queue backend (Cloud Tasks' retry-count
    /// header isn't available on the local queue). `None` if the run already
    /// `CompleteSuccess` — that redelivery must be ignored, not resurrected.
    /// A `SubmissionFailed` run never reached a worker and is also ignored.
    pub async fn begin_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
    ) -> anyhow::Result<Option<i32>> {
        let status = self
            .run_tracker
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

        self.run_tracker
            .update_run(envelope, TaskRunStatus::InProgress, attempt)
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
    ) -> anyhow::Result<TaskRunStatus> {
        match run_result {
            Ok(()) => {
                self.run_tracker
                    .update_run(envelope, TaskRunStatus::CompleteSuccess, attempt)
                    .await?;
                Ok(TaskRunStatus::CompleteSuccess)
            }
            Err(err) => {
                let status_code = if decide_will_retry(err, attempt, self.max_attempts_per_run) {
                    TaskRunStatus::ErrorWillRetry
                } else {
                    TaskRunStatus::CompleteFailure
                };

                self.run_tracker
                    .update_run(envelope, status_code, attempt)
                    .await?;

                tracing::warn!(
                    envelope = ?envelope,
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

    // TODO should we return something better than the internal Model instances here??
    pub async fn query_latest_status_for_entities(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_ids: &[i32],
    ) -> anyhow::Result<Vec<model::task_run_status::Model>> {
        self.run_tracker
            .query_latest_status_for_entities(user_id, entity_type, entity_ids)
            .await
    }
}

#[derive(Default)]
pub struct TaskMasterBuilder {
    db: Option<DbHandle>,
    notifier: Option<Arc<dyn ServerEventNotifier>>,
    max_attempts: Option<i32>,
    illuminate_queue: Option<Box<dyn TaskQueue<IlluminationTask>>>,
    search_index_queue: Option<Box<dyn TaskQueue<SearchIndexTask>>>,
    spark_queue: Option<Box<dyn TaskQueue<SparkTask>>>,
}

impl TaskMasterBuilder {
    pub fn db(mut self, db: DbHandle) -> Self {
        self.db = Some(db);
        self
    }
    pub fn notifier(mut self, notifier: Option<Arc<dyn ServerEventNotifier>>) -> Self {
        self.notifier = notifier;
        self
    }

    /// Maximum number of attempts per task. See `Config::task_max_attempts`.
    pub fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }

    pub fn illuminate_queue(
        mut self,
        illuminate_queue: impl TaskQueue<IlluminationTask> + 'static,
    ) -> Self {
        self.illuminate_queue = Some(Box::new(illuminate_queue));
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
        #[cfg(not(test))]
        let (Some(illuminate_queue), Some(search_index_queue), Some(spark_queue)) = (
            self.illuminate_queue,
            self.search_index_queue,
            self.spark_queue,
        ) else {
            anyhow::bail!("TaskMaster requires all task queues");
        };

        #[cfg(test)]
        let illuminate_queue = self
            .illuminate_queue
            .unwrap_or_else(|| Box::new(TestNoopQueue::default()));
        #[cfg(test)]
        let search_index_queue = self
            .search_index_queue
            .unwrap_or_else(|| Box::new(TestNoopQueue::default()));
        #[cfg(test)]
        let spark_queue = self
            .spark_queue
            .unwrap_or_else(|| Box::new(TestNoopQueue::default()));

        Ok(TaskMaster {
            run_tracker: TaskRunTracker::new(db, self.notifier),
            // Mirrors `Config::task_max_attempts` so a builder that forgets
            // `.max_attempts(..)` behaves like production rather than disabling retries.
            max_attempts_per_run: self.max_attempts.unwrap_or(3).max(1),
            illuminate_queue,
            search_index_queue,
            spark_queue,
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct TestNoopQueue<T>(std::marker::PhantomData<T>);

#[cfg(test)]
impl<T> Default for TestNoopQueue<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl<T: Task> TaskQueue<T> for TestNoopQueue<T> {
    async fn enqueue(&self, _envelope: TaskEnvelope<T>) -> anyhow::Result<()> {
        Ok(())
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
        let row = status_row(TaskRunStatus::ErrorWillRetry, 2);

        assert_eq!(decide_next_attempt(Some(&row)), Some(3));
    }

    #[test]
    fn completed_task_is_not_resurrected() {
        let row = status_row(TaskRunStatus::CompleteSuccess, 1);

        assert_eq!(
            decide_next_attempt(Some(&row)),
            None,
            "an at-least-once redelivery of finished work must be ignored"
        );
    }

    #[test]
    fn failed_submission_is_not_a_worker_attempt() {
        let row = status_row(TaskRunStatus::SubmissionFailed, 0);

        assert_eq!(decide_next_attempt(Some(&row)), None);
    }

    #[test]
    fn submission_starts_at_run_one_when_never_run() {
        assert_eq!(decide_next_run(None), Some(1));
    }

    #[test]
    fn submission_is_refused_while_a_run_is_in_flight() {
        for status in [
            TaskRunStatus::Queued,
            TaskRunStatus::InProgress,
            TaskRunStatus::ErrorWillRetry,
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
        for status in [
            TaskRunStatus::CompleteSuccess,
            TaskRunStatus::CompleteFailure,
        ] {
            let row = status_row_of_run(status, 1, 3);

            assert_eq!(
                decide_next_run(Some(&row)),
                Some(4),
                "{status} is settled, so the next run is permitted"
            );
        }
    }

    #[test]
    fn submission_failed_allows_a_new_run() {
        let row = status_row_of_run(TaskRunStatus::SubmissionFailed, 0, 4);

        assert_eq!(decide_next_run(Some(&row)), Some(5));
    }

    #[test]
    fn zero_max_attempts_is_effectively_one_attempt() {
        let err = api::ApiError::internal(anyhow::anyhow!("transient"));

        assert!(!decide_will_retry(&err, 1, 0));
    }

    /// A `task_run_status` row with only the fields the pure helpers read set to
    /// meaningful values.
    fn status_row(status: TaskRunStatus, attempts: i32) -> model::task_run_status::Model {
        status_row_of_run(status, attempts, 1)
    }

    fn status_row_of_run(
        status: TaskRunStatus,
        attempts: i32,
        run: i32,
    ) -> model::task_run_status::Model {
        model::task_run_status::Model {
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
            captures.push(envelope.task.capture_id);
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
            .illuminate_queue(queue)
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let recorded = captures
            .lock()
            .expect("captures mutex should not be poisoned")
            .clone();
        assert_eq!(recorded, vec![42]);
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
            .illuminate_queue(queue)
            .build()
            .expect("build should succeed with a db");

        let result = service
            .submit_illuminate(1, IlluminationTask { capture_id: 9 })
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[42])
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, TaskRunStatus::Queued.as_i32());
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::clone(&captures),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let first = service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("first submit should succeed");
        let second = service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("second submit should be answered, not error");

        assert_eq!(first, SubmitOutcome::Enqueued { run: 1 });
        assert_eq!(second, SubmitOutcome::RefusedAlreadyInFlight);
        assert_eq!(
            captures.lock().unwrap().len(),
            1,
            "the duplicate must not reach the queue"
        );
    }

    #[tokio::test]
    async fn concurrent_duplicate_submissions_enqueue_exactly_one_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let captures = Arc::new(Mutex::new(Vec::new()));
        let service = Arc::new(
            TaskMaster::builder()
                .db(db.handle())
                .illuminate_queue(RecordingQueue {
                    captures: Arc::clone(&captures),
                    fail: false,
                })
                .build()
                .expect("build should succeed with a db"),
        );
        let (first, second) = tokio::join!(
            service.submit_illuminate(1, IlluminationTask { capture_id: 42 }),
            service.submit_illuminate(1, IlluminationTask { capture_id: 42 }),
        );
        let outcomes = [first.unwrap(), second.unwrap()];

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == SubmitOutcome::Enqueued { run: 1 })
                .count(),
            1,
            "only one concurrent submit may enqueue run one"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == SubmitOutcome::RefusedAlreadyInFlight)
                .count(),
            1
        );
        assert_eq!(captures.lock().unwrap().as_slice(), &[42]);
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1 exhausts (incomplete, settled).
        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            TaskRunStatus::CompleteFailure,
            "max_attempts=1 means the first failure is terminal"
        );

        // Run 2 is queued (in flight).
        let rerun = service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("rerun submit should succeed");
        assert_eq!(rerun, SubmitOutcome::Enqueued { run: 2 });

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[42])
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1, "only the latest run is reported");
        assert_eq!(rows[0].run, 2);
        assert_eq!(rows[0].status_code, TaskRunStatus::Queued.as_i32());
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1 exhausts and stays incomplete.
        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            .query_latest_status_for_entities(1, "capture", &[42])
            .await
            .expect("query should succeed");

        assert_eq!(rows.len(), 1, "only the latest run is reported");
        assert_eq!(rows[0].run, 2, "the completed rerun supersedes run 1");
        assert_eq!(rows[0].status_code, TaskRunStatus::CompleteSuccess.as_i32());
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        // Run 1: two attempts, then exhaust.
        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
        assert_eq!(outcome, TaskRunStatus::ErrorWillRetry);

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
            TaskRunStatus::CompleteFailure,
            "run 1 has now spent its budget"
        );

        // Run 2 begins at attempt 1 again, even though run 1 used attempts.
        service
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 7 };
        service
            .submit_illuminate(1, task.clone())
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
        assert_eq!(outcome, TaskRunStatus::CompleteSuccess);

        // The completed row is reported: a caller must be able to see that its
        // work finished.
        let rows = service
            .query_latest_status_for_entities(1, "capture", &[7])
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, TaskRunStatus::CompleteSuccess.as_i32());
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 9 };
        service
            .submit_illuminate(1, task.clone())
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
        assert_eq!(outcome, TaskRunStatus::ErrorWillRetry);

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
        assert_eq!(outcome, TaskRunStatus::CompleteFailure);

        // The exhausted run is still reported (the user should see it).
        let rows = service
            .query_latest_status_for_entities(1, "capture", &[9])
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status_code, TaskRunStatus::CompleteFailure.as_i32());
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        let task = IlluminationTask { capture_id: 11 };
        service
            .submit_illuminate(1, task.clone())
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
    async fn query_latest_status_for_entities_spans_requested_entities() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };

        let service = TaskMaster::builder()
            .db(db.handle())
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        for capture_id in [1, 2, 3] {
            service
                .submit_illuminate(1, IlluminationTask { capture_id })
                .await
                .expect("submit should succeed");
        }

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[1, 2, 3])
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 3);

        // A different user cannot see these entities.
        let other = service
            .query_latest_status_for_entities(2, "capture", &[1, 2, 3])
            .await
            .expect("query should succeed");
        assert!(other.is_empty(), "queries must be scoped by user_id");
    }

    /// A service with an illuminate queue that records (or fails) enqueues.
    fn service(db: &crate::test_support::test_db::TestDb, queue_fails: bool) -> TaskMaster {
        TaskMaster::builder()
            .db(db.handle())
            .illuminate_queue(RecordingQueue {
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

    /// The row is written before the enqueue, then marked `SubmissionFailed` if
    /// the queue rejects it, so a later submission can start a new run.
    #[tokio::test]
    async fn failed_enqueue_records_submission_failed() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let service = service(&db, true);

        let result = service
            .submit_illuminate(1, IlluminationTask { capture_id: 9 })
            .await;
        assert!(result.is_err(), "the enqueue error must propagate");

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[9])
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].status_code,
            TaskRunStatus::SubmissionFailed.as_i32()
        );

        let retry = service
            .submit_illuminate(1, IlluminationTask { capture_id: 9 })
            .await;
        assert!(retry.is_err(), "the test queue is still configured to fail");

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[9])
            .await
            .expect("query should succeed");
        assert_eq!(rows.len(), 1, "only the latest run is returned");
        assert_eq!(rows[0].run, 2);
    }

    /// Only `CompleteSuccess` is protected from redelivery. An exhausted run is not,
    /// so a redelivery would move it back to `InProgress`.
    ///
    /// Unreachable today: we ack `CompleteFailure` with a 2xx, so Cloud Tasks
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
            .illuminate_queue(RecordingQueue {
                captures: Arc::new(Mutex::new(Vec::new())),
                fail: false,
            })
            .build()
            .expect("build should succeed with a db");

        service
            .submit_illuminate(1, IlluminationTask { capture_id: 5 })
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
        assert_eq!(outcome, TaskRunStatus::CompleteFailure);

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
            .submit_illuminate(1, IlluminationTask { capture_id: 42 })
            .await
            .expect("submit should succeed");

        let rows = service
            .query_latest_status_for_entities(1, "capture", &[42])
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
            .submit_illuminate(1, IlluminationTask { capture_id: 3 })
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
