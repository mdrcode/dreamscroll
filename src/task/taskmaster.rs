use crate::api;
use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::ingest::IngestTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::model;

use super::*;

/// The primary entry point for manipulating `Task` instances.
///
/// `TaskMaster` owns the backend queues **and** the `task_status` table. It is
/// one of only two structs allowed to touch `task_status` directly (the other
/// is `TaskWatcher`, the future LISTEN/NOTIFY thread). Everything else in the
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
/// `db` is optional: when absent, `TaskMaster` runs in **enqueue-only** mode
/// (no `task_status` writes). This is used by util commands and tests that
/// don't want background-task bookkeeping.
///
/// Not Clone, share it via Arc.
pub struct TaskMaster {
    status: TaskStatusRecorder,
    max_attempts: i32,
    ingest_queue: Option<Box<dyn TaskQueue<IngestTask>>>,
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

/// Compute the 1-based attempt number for a task about to start.
///
/// Returns `None` when the task is already `Completed`: Cloud Tasks delivers at
/// least once, so a redelivery of finished work must not resurrect the row back
/// to `InProgress` (which would show a spurious "in progress" blip to any client
/// watching the task).
fn next_attempt_number(snapshot: Option<&TaskStatusSnapshot>) -> Option<i32> {
    match snapshot {
        Some(snapshot) if snapshot.status == StatusCode::Completed => None,
        Some(snapshot) => Some(snapshot.attempts + 1),
        None => Some(1),
    }
}

impl TaskMaster {
    pub fn builder() -> TaskMasterBuilder {
        TaskMasterBuilder::default()
    }

    pub async fn submit_ingest(&self, user_id: i32, task: IngestTask) -> anyhow::Result<()> {
        self.submit_inner(self.ingest_queue.as_ref(), user_id, task)
            .await
    }

    pub async fn submit_illumination(
        &self,
        user_id: i32,
        task: IlluminationTask,
    ) -> anyhow::Result<()> {
        self.submit_inner(self.illumination_queue.as_ref(), user_id, task)
            .await
    }

    pub async fn submit_spark(&self, user_id: i32, task: SparkTask) -> anyhow::Result<()> {
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
    ) -> anyhow::Result<()> {
        self.submit_inner(self.search_index_queue.as_ref(), user_id, task)
            .await
    }

    async fn submit_inner<T: Task>(
        &self,
        queue: Option<&Box<dyn TaskQueue<T>>>,
        user_id: i32,
        task: T,
    ) -> anyhow::Result<()> {
        let task_type = T::task_type();
        let envelope = TaskEnvelope::new(user_id, task);

        let Some(queue) = queue else {
            tracing::warn!(
                envelope = ?envelope,
                "{} submitted but no queue configured, skipping enqueue.",
                task_type,
            );
            return Ok(());
        };

        // Record `Queued` before enqueueing, since `enqueue` moves the envelope.
        self.status.record(&envelope, StatusCode::Queued, 0).await?;

        queue.enqueue(envelope.clone()).await.inspect_err(|err| {
            tracing::error!(
                queue = ?queue,
                envelope = ?envelope,
                error = ?err,
                "Failed to enqueue task",
            )
        })?;

        Ok(())
    }

    /// Mark an attempt as starting and return its 1-based attempt number.
    ///
    /// The attempt number is derived from the persisted `attempts` count, so it
    /// survives worker restarts and works identically for every queue backend
    /// (unlike Cloud Tasks' retry-count header, which the local queue lacks).
    ///
    /// Returns `None` if the task is already `Completed`. Cloud Tasks delivers
    /// at least once, so a redelivery of finished work must not resurrect it
    /// back to `InProgress` (which would show a spurious "in progress" blip to
    /// any client watching the task).
    pub async fn begin_attempt<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
    ) -> anyhow::Result<Option<i32>> {
        let snapshot = self.status.query_snapshot(&envelope.envelope_id).await?;

        let Some(attempt) = next_attempt_number(snapshot.as_ref()) else {
            tracing::info!(
                envelope_id = %envelope.envelope_id,
                "Ignoring attempt for already-completed task"
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

    /// Record a status transition for a task. Upserts the row keyed by
    /// `envelope_id`. `attempts` is the attempt count at the time of this
    /// transition.
    ///
    /// Private on purpose: external callers must use `begin_attempt`/`finish_attempt`,
    /// which keep `attempts` and the retry decision consistent with the status.
    async fn update_status<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: StatusCode,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.status.record(envelope, status, attempts).await
    }

    /// Query the *incomplete* task statuses recorded against a given entity,
    /// e.g. all tasks (`illuminate`, `ingest`, `search_index`, ...) that
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
    ingest_queue: Option<Box<dyn TaskQueue<IngestTask>>>,
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

    pub fn ingest_queue(mut self, ingest_queue: impl TaskQueue<IngestTask> + 'static) -> Self {
        self.ingest_queue = Some(Box::new(ingest_queue));
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

    pub fn build(self) -> TaskMaster {
        TaskMaster {
            status: TaskStatusRecorder::new(self.db),
            // Mirrors `Config::task_max_attempts`'s default so a builder that
            // forgets `.max_attempts(..)` behaves like production rather than
            // silently disabling retries.
            max_attempts: self.max_attempts.unwrap_or(3).max(1),
            ingest_queue: self.ingest_queue,
            illumination_queue: self.illumination_queue,
            search_index_queue: self.search_index_queue,
            spark_queue: self.spark_queue,
        }
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
        let snapshot = TaskStatusSnapshot {
            status: StatusCode::ErrorWillRetry,
            attempts: 2,
        };

        assert_eq!(next_attempt_number(Some(&snapshot)), Some(3));
    }

    #[test]
    fn completed_task_is_not_resurrected() {
        let snapshot = TaskStatusSnapshot {
            status: StatusCode::Completed,
            attempts: 1,
        };

        assert_eq!(
            next_attempt_number(Some(&snapshot)),
            None,
            "an at-least-once redelivery of finished work must be ignored"
        );
    }

    #[derive(Debug, Clone)]
    struct RecordingQueue {
        captures: Arc<Mutex<Vec<i32>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl TaskQueue<IngestTask> for RecordingQueue {
        async fn enqueue(&self, envelope: TaskEnvelope<IngestTask>) -> anyhow::Result<()> {
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
    async fn submit_ingest_enqueues_task() {
        let captures = Arc::new(Mutex::new(Vec::new()));
        let queue = RecordingQueue {
            captures: Arc::clone(&captures),
            fail: false,
        };

        let service = TaskMaster::builder().ingest_queue(queue).build();

        service
            .submit_ingest(1, IngestTask { capture_id: 42 })
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
        let service = TaskMaster::builder().build();

        service
            .submit_ingest(1, IngestTask { capture_id: 7 })
            .await
            .expect("submit should be a no-op when queue is absent");
    }

    #[tokio::test]
    async fn submit_propagates_enqueue_error() {
        let queue = RecordingQueue {
            captures: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        };
        let service = TaskMaster::builder().ingest_queue(queue).build();

        let result = service.submit_ingest(1, IngestTask { capture_id: 9 }).await;
        assert!(result.is_err());
    }
}
