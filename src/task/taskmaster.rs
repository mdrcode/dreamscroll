use crate::database::DbHandle;
use crate::logic::illuminate::IlluminationTask;
use crate::logic::ingest::IngestTask;
use crate::logic::search_index::SearchIndexTask;
use crate::logic::spark::SparkTask;
use crate::model::task_status::Status;

use super::*;

/// The primary entry point for manipulating `Task` instances.
///
/// `TaskMaster` owns the backend queues **and** the `task_status` table. It is
/// one of only two structs allowed to touch `task_status` directly (the other
/// is `TaskWatcher`, the future LISTEN/NOTIFY thread). Everything else in the
/// system talks to tasks through this API:
///
/// - `submit` — enqueue + record a `Queued` row.
/// - `update_status` — record a status transition (workers call this).
/// - `query_status` — read current status (replay / polling).
///
/// `db` is optional: when absent, `TaskMaster` runs in **enqueue-only** mode
/// (no `task_status` writes). This is used by util commands and tests that
/// don't want background-task bookkeeping.
///
/// Not Clone, share it via Arc.
pub struct TaskMaster {
    status: TaskStatusRecorder,
    ingest_queue: Option<Box<dyn TaskQueue<IngestTask>>>,
    illumination_queue: Option<Box<dyn TaskQueue<IlluminationTask>>>,
    search_index_queue: Option<Box<dyn TaskQueue<SearchIndexTask>>>,
    spark_queue: Option<Box<dyn TaskQueue<SparkTask>>>,
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
        let envelope = TaskEnvelope {
            user_id,
            task_id: make_task_id(user_id, &task),
            task: Some(task),
        };
        let task_id = envelope.task_id.clone();

        let Some(queue) = queue else {
            tracing::warn!(
                wrapped = ?envelope,
                "{} submitted but no queue configured, skipping enqueue.",
                task_type,
            );
            return Ok(());
        };

        // Record `Queued` before enqueueing, since `enqueue` moves the envelope.
        self.status.record(&envelope, Status::Queued, 0).await?;

        queue.enqueue(envelope).await.inspect_err(|err| {
            tracing::error!(
                queue = ?queue,
                task_id,
                error = ?err,
                "Failed to enqueue {} task: {}",
                task_type,
                err,
            )
        })?;

        Ok(())
    }

    /// Record a status transition for a task. Upserts the row keyed by
    /// (task_type, task_id). `attempts` is the attempt count at the
    /// time of this transition.
    pub async fn update_status<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: Status,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.status.record(envelope, status, attempts).await
    }

    /// Query the current status row for a task, if one exists.
    ///
    /// Returns `None` when there's no DB (enqueue-only mode) or no row yet.
    pub async fn query_status(
        &self,
        task_type: &str,
        _user_id: i32,
        task_id: &str,
    ) -> anyhow::Result<Option<Status>> {
        self.status.query(task_type, task_id).await
    }
}

#[derive(Default)]
pub struct TaskMasterBuilder {
    db: Option<DbHandle>,
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

    pub fn build(self) -> TaskMaster {
        TaskMaster {
            status: TaskStatusRecorder::new(self.db),
            ingest_queue: self.ingest_queue,
            illumination_queue: self.illumination_queue,
            search_index_queue: self.search_index_queue,
            spark_queue: self.spark_queue,
        }
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
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone)]
    struct RecordingQueue {
        captures: Arc<Mutex<Vec<i32>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl TaskQueue<IngestTask> for RecordingQueue {
        async fn enqueue(&self, wrapped: TaskEnvelope<IngestTask>) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("enqueue failed")
            }

            let mut captures = self
                .captures
                .lock()
                .expect("RecordingQueue captures mutex should not be poisoned");
            captures.push(wrapped.task.unwrap().capture_id);
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
