use std::sync::Arc;

use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::str::FromStr;

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
#[derive(Clone)]
pub struct TaskMaster {
    db: Option<DbHandle>,
    ingest_queue: Option<Arc<dyn TaskQueue<Task = IngestTask>>>,
    illumination_queue: Option<Arc<dyn TaskQueue<Task = IlluminationTask>>>,
    search_index_queue: Option<Arc<dyn TaskQueue<Task = SearchIndexTask>>>,
    spark_queue: Option<Arc<dyn TaskQueue<Task = SparkTask>>>,
}

impl TaskMaster {
    pub fn builder() -> TaskMasterBuilder {
        TaskMasterBuilder::default()
    }

    /// Submit a task for execution: enqueue on the matching backend queue and
    /// record a `Queued` row in `task_status`.
    ///
    /// If the queue for this task type is not configured, this is a no-op
    /// (warn + return Ok) — matching the old `Beacon` behavior.
    pub async fn submit(&self, task: &Task) -> anyhow::Result<()> {
        match task {
            Task::Ingest { capture_id, .. } => {
                let Some(queue) = self.ingest_queue.as_ref() else {
                    tracing::warn!(
                        capture_id,
                        "Ingest requested but no ingest queue configured, skipping enqueue."
                    );
                    return Ok(());
                };
                queue.enqueue(IngestTask { capture_id: *capture_id }).await.inspect_err(
                    |err| tracing::error!(queue = ?queue, capture_id, error = ?err, "Failed to enqueue capture for ingest: {}", err),
                )?;
            }
            Task::Illuminate { capture_id, .. } => {
                let Some(queue) = self.illumination_queue.as_ref() else {
                    tracing::warn!(
                        capture_id,
                        "Illumination requested but no illumination queue configured, skipping enqueue."
                    );
                    return Ok(());
                };
                queue
                    .enqueue(IlluminationTask { capture_id: *capture_id })
                    .await
                    .inspect_err(|err| {
                        tracing::error!(
                            queue = ?queue,
                            capture_id,
                            error = ?err,
                            "Failed to enqueue capture for illumination: {}",
                            err
                        )
                    })?;
            }
            Task::Spark { capture_ids, .. } => {
                if capture_ids.is_empty() {
                    anyhow::bail!("submit(Spark) requires at least one capture_id");
                }
                let Some(queue) = self.spark_queue.as_ref() else {
                    tracing::warn!(
                        capture_ids = ?capture_ids,
                        "Spark requested but no spark queue configured, skipping enqueue."
                    );
                    return Ok(());
                };
                queue
                    .enqueue(SparkTask {
                        capture_ids: capture_ids.clone(),
                    })
                    .await
                    .inspect_err(|err| {
                        tracing::error!(
                            queue = ?queue,
                            capture_ids = ?capture_ids,
                            error = ?err,
                            "Failed to enqueue captures for spark: {}",
                            err
                        )
                    })?;
            }
            Task::SearchIndex { capture_id, .. } => {
                let Some(queue) = self.search_index_queue.as_ref() else {
                    tracing::warn!(
                        capture_id,
                        "Search index requested but no search index queue configured, skipping enqueue."
                    );
                    return Ok(());
                };
                queue
                    .enqueue(SearchIndexTask { capture_id: *capture_id })
                    .await
                    .inspect_err(|err| {
                        tracing::error!(
                            queue = ?queue,
                            capture_id,
                            error = ?err,
                            "Failed to enqueue capture for search indexing: {}",
                            err
                        )
                    })?;
            }
        }

        // Record the Queued row (best-effort; enqueue is the critical path).
        self.record_status(task, Status::Queued, 0).await?;

        Ok(())
    }

    /// Record a status transition for a task. Upserts the row keyed by
    /// (task_type, task_id, run_id). `attempts` is the attempt count at the
    /// time of this transition.
    pub async fn update_status(
        &self,
        task: &Task,
        status: Status,
        attempts: i32,
    ) -> anyhow::Result<()> {
        self.record_status(task, status, attempts).await
    }

    /// Query the current status row for a task, if one exists.
    ///
    /// Returns `None` when there's no DB (enqueue-only mode) or no row yet.
    pub async fn query_status(&self, task: &Task) -> anyhow::Result<Option<Status>> {
        let Some(db) = self.db.as_ref() else {
            return Ok(None);
        };

        let row = crate::model::task_status::Entity::find()
            .filter(crate::model::task_status::Column::TaskType.eq(task.task_type()))
            .filter(crate::model::task_status::Column::TaskId.eq(task.task_id()))
            .filter(crate::model::task_status::Column::RunId.eq(1))
            .one(&db.conn)
            .await?;

        if let Some(r) = row {
            Ok(Some(Status::from_str(&r.status)?))
        } else {
            Ok(None)
        }
    }

    /// Shared upsert used by `submit` and `update_status`. No-op without a DB.
    async fn record_status(
        &self,
        task: &Task,
        status: Status,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let Some(db) = self.db.as_ref() else {
            return Ok(());
        };

        let existing = crate::model::task_status::Entity::find()
            .filter(crate::model::task_status::Column::TaskType.eq(task.task_type()))
            .filter(crate::model::task_status::Column::TaskId.eq(task.task_id()))
            .filter(crate::model::task_status::Column::RunId.eq(1))
            .one(&db.conn)
            .await?;

        if let Some(row) = existing {
            let mut active: crate::model::task_status::ActiveModel = row.into();
            active.status = Set(status.as_str().to_string());
            active.attempts = Set(attempts);
            active.updated_at = Set(chrono::Utc::now());
            active.update(&db.conn).await?;
        } else {
            crate::model::task_status::ActiveModel::builder()
                .set_task_type(task.task_type())
                .set_task_id(task.task_id())
                .set_run_id(1)
                .set_user_id(task.user_id())
                .set_status(status.as_str().to_string())
                .set_attempts(attempts)
                .set_background(false)
                .save(&db.conn)
                .await?;
        }

        Ok(())
    }
}

#[derive(Default)]
pub struct TaskMasterBuilder {
    db: Option<DbHandle>,
    ingest_queue: Option<Arc<dyn TaskQueue<Task = IngestTask>>>,
    illumination_queue: Option<Arc<dyn TaskQueue<Task = IlluminationTask>>>,
    search_index_queue: Option<Arc<dyn TaskQueue<Task = SearchIndexTask>>>,
    spark_queue: Option<Arc<dyn TaskQueue<Task = SparkTask>>>,
}

impl TaskMasterBuilder {
    pub fn db(mut self, db: DbHandle) -> Self {
        self.db = Some(db);
        self
    }

    pub fn ingest_queue(
        mut self,
        ingest_queue: impl TaskQueue<Task = IngestTask> + 'static,
    ) -> Self {
        self.ingest_queue = Some(Arc::new(ingest_queue));
        self
    }

    pub fn illumination_queue(
        mut self,
        illumination_queue: impl TaskQueue<Task = IlluminationTask> + 'static,
    ) -> Self {
        self.illumination_queue = Some(Arc::new(illumination_queue));
        self
    }

    pub fn build(self) -> TaskMaster {
        TaskMaster {
            db: self.db,
            ingest_queue: self.ingest_queue,
            illumination_queue: self.illumination_queue,
            search_index_queue: self.search_index_queue,
            spark_queue: self.spark_queue,
        }
    }

    pub fn search_index_queue(
        mut self,
        search_index_queue: impl TaskQueue<Task = SearchIndexTask> + 'static,
    ) -> Self {
        self.search_index_queue = Some(Arc::new(search_index_queue));
        self
    }

    pub fn spark_queue(mut self, spark_queue: impl TaskQueue<Task = SparkTask> + 'static) -> Self {
        self.spark_queue = Some(Arc::new(spark_queue));
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
    impl TaskQueue for RecordingQueue {
        type Task = IngestTask;

        async fn enqueue(&self, task: Self::Task) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("enqueue failed")
            }

            let mut captures = self
                .captures
                .lock()
                .expect("RecordingQueue captures mutex should not be poisoned");
            captures.push(task.capture_id);
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
            .submit(&Task::Ingest { user_id: 1, capture_id: 42 })
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
            .submit(&Task::Ingest { user_id: 1, capture_id: 7 })
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

        let result = service.submit(&Task::Ingest { user_id: 1, capture_id: 9 }).await;
        assert!(result.is_err());
    }
}