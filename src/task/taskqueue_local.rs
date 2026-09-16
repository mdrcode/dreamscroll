use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

use super::*;

type TaskHandlerFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'static>>;
type TaskHandler<TTask> =
    Arc<dyn Fn(TaskEnvelope<TTask>) -> TaskHandlerFuture + Send + Sync + 'static>;

pub struct LocalTaskQueue<TTask: Task> {
    inner: Arc<LocalTaskQueueInner<TTask>>,
    _task: PhantomData<TTask>,
}

struct LocalTaskQueueInner<TTask: Task> {
    task_sender: mpsc::UnboundedSender<TaskEnvelope<TTask>>,
    max_concurrent_tasks: usize,
    dispatcher_handle: Mutex<Option<JoinHandle<()>>>,
}

impl<TTask: Task> Clone for LocalTaskQueue<TTask> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _task: PhantomData,
        }
    }
}

impl<TTask: Task> fmt::Debug for LocalTaskQueue<TTask> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTaskQueue")
            .field("max_concurrent_tasks", &self.inner.max_concurrent_tasks)
            .finish()
    }
}

impl<TTask> LocalTaskQueue<TTask>
where
    TTask: Task + Send + Sync + 'static,
{
    pub fn connect<F, Fut>(max_concurrent_tasks: usize, task_handler: F) -> Self
    where
        F: Fn(TaskEnvelope<TTask>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let max_concurrent_tasks = max_concurrent_tasks.max(1);

        let handler: TaskHandler<TTask> =
            Arc::new(move |task: TaskEnvelope<TTask>| -> TaskHandlerFuture {
                Box::pin(task_handler(task))
            });
        let semaphore = Arc::new(Semaphore::new(max_concurrent_tasks));
        let (task_sender, mut task_receiver) = mpsc::unbounded_channel::<TaskEnvelope<TTask>>();

        // One dispatcher receives tasks in FIFO order and fan-outs execution to workers.
        // A semaphore bounds worker concurrency to max_concurrent_tasks.
        let dispatcher_handle = tokio::spawn(async move {
            Self::run_dispatcher(&mut task_receiver, semaphore, handler).await;
        });

        let inner = Arc::new(LocalTaskQueueInner {
            task_sender,
            max_concurrent_tasks,
            dispatcher_handle: Mutex::new(Some(dispatcher_handle)),
        });

        Self {
            inner,
            _task: PhantomData,
        }
    }

    async fn run_dispatcher(
        task_receiver: &mut mpsc::UnboundedReceiver<TaskEnvelope<TTask>>,
        semaphore: Arc<Semaphore>,
        handler: TaskHandler<TTask>,
    ) {
        while let Some(task) = task_receiver.recv().await {
            // Block until a permit is available, limiting concurrency
            // number of permits total == max_concurrent_tasks
            // number of permits avail == max_concurrent_tasks - running tasks
            let permit = match Arc::clone(&semaphore).acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => break,
            };

            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let _permit = permit; // releases when dropped
                if let Err(err) = (handler)(task).await {
                    tracing::error!(error = ?err, "Local task execution failed");
                }
            });
        }
    }
}

impl<TTask: Task> Drop for LocalTaskQueue<TTask> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }

        let mut maybe_dispatcher_handle = self
            .inner
            .dispatcher_handle
            .lock()
            .expect("LocalTaskQueue dispatcher handle mutex should not be poisoned");

        if let Some(dispatcher_handle) = maybe_dispatcher_handle.take() {
            dispatcher_handle.abort();
        }
    }
}

#[async_trait::async_trait]
impl<TTask> TaskQueue<TTask> for LocalTaskQueue<TTask>
where
    TTask: Task + Send + Sync + 'static,
{
    async fn enqueue(&self, envelope: TaskEnvelope<TTask>) -> anyhow::Result<()> {
        let type_name = std::any::type_name::<TTask>()
            .rsplit("::")
            .next()
            .unwrap_or("Task");
        let task_str = format!(
            "{} {}",
            type_name,
            serde_json::to_string(&envelope)
                .unwrap_or_else(|_| "<serialization error>".to_string())
        );
        tracing::info!(task = %task_str, "Enqueuing task into LocalTaskQueue");
        self.inner
            .task_sender
            .send(envelope)
            .map_err(|_| anyhow!("Cannot enqueue into LocalTaskQueue after shutdown"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::{Mutex as AsyncMutex, Notify};
    use tokio::time::{Duration, sleep};

    use super::*;

    #[derive(Debug, Clone, serde::Serialize)]
    struct TestTask {
        id: i32,
    }

    impl Task for TestTask {
        fn entity_type() -> &'static str {
            "capture"
        }

        fn entity_id(&self) -> i32 {
            self.id
        }

        fn task_type() -> &'static str {
            "test"
        }
    }

    fn envelope(id: i32) -> TaskEnvelope<TestTask> {
        TaskEnvelope::new(1, TestTask { id }, 1)
    }

    /// Poll until `predicate` holds, or fail the test on timeout.
    async fn wait_until(label: &str, mut predicate: impl FnMut() -> bool) {
        let deadline = Duration::from_secs(2);
        let start = std::time::Instant::now();

        while !predicate() {
            assert!(start.elapsed() < deadline, "timed out waiting for: {label}");
            sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn executes_enqueued_tasks() -> anyhow::Result<()> {
        let seen = Arc::new(AsyncMutex::new(Vec::new()));
        let seen_for_worker = Arc::clone(&seen);

        let queue = LocalTaskQueue::connect(4, move |task: TaskEnvelope<TestTask>| {
            let seen = Arc::clone(&seen_for_worker);
            async move {
                seen.lock().await.push(task.task.unwrap().id);
                Ok(())
            }
        });

        queue.enqueue(envelope(1)).await?;
        queue.enqueue(envelope(2)).await?;

        wait_until("both tasks to run", || {
            seen.try_lock().map(|s| s.len() == 2).unwrap_or(false)
        })
        .await;

        let values = seen.lock().await.clone();
        assert!(values.contains(&1) && values.contains(&2));
        Ok(())
    }

    /// The semaphore must both *allow* parallelism and *bound* it. Asserting
    /// only the lower bound would pass even if the semaphore were removed.
    #[tokio::test]
    async fn concurrency_is_parallel_but_bounded() -> anyhow::Result<()> {
        const LIMIT: usize = 3;
        const TASKS: i32 = 9;

        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));

        let (active_w, max_w, done_w) = (
            Arc::clone(&active),
            Arc::clone(&max_active),
            Arc::clone(&done),
        );

        let queue = LocalTaskQueue::connect(LIMIT, move |_task: TaskEnvelope<TestTask>| {
            let (active, max_active, done) = (
                Arc::clone(&active_w),
                Arc::clone(&max_w),
                Arc::clone(&done_w),
            );

            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);

                sleep(Duration::from_millis(20)).await;

                active.fetch_sub(1, Ordering::SeqCst);
                done.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        for id in 0..TASKS {
            queue.enqueue(envelope(id)).await?;
        }

        wait_until("all tasks to finish", || {
            done.load(Ordering::SeqCst) == TASKS as usize
        })
        .await;

        let peak = max_active.load(Ordering::SeqCst);
        assert!(peak >= 2, "expected parallelism, saw peak {peak}");
        assert!(
            peak <= LIMIT,
            "concurrency must not exceed the limit: peak {peak} > {LIMIT}"
        );
        Ok(())
    }

    /// Dropping the last handle aborts the dispatcher, so tasks still queued
    /// behind the in-flight one are never started.
    #[tokio::test]
    async fn shutdown_does_not_drain_the_queue() -> anyhow::Result<()> {
        let started = Arc::new(AtomicUsize::new(0));
        let processed = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());

        let (started_w, processed_w, release_w) = (
            Arc::clone(&started),
            Arc::clone(&processed),
            Arc::clone(&release),
        );

        // Concurrency 1, so only the first task can be in flight.
        let queue = LocalTaskQueue::connect(1, move |_task: TaskEnvelope<TestTask>| {
            let (started, processed, release) = (
                Arc::clone(&started_w),
                Arc::clone(&processed_w),
                Arc::clone(&release_w),
            );

            async move {
                started.fetch_add(1, Ordering::SeqCst);
                release.notified().await;
                processed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        queue.enqueue(envelope(1)).await?;
        queue.enqueue(envelope(2)).await?;
        queue.enqueue(envelope(3)).await?;

        wait_until("the first task to start", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        drop(queue);
        release.notify_one();

        // Give the aborted dispatcher a chance to (incorrectly) dispatch more.
        sleep(Duration::from_millis(50)).await;

        assert_eq!(
            processed.load(Ordering::SeqCst),
            1,
            "tasks queued behind the in-flight one must not run after shutdown"
        );
        Ok(())
    }

    /// A failing task must not stop the queue from processing later ones.
    #[tokio::test]
    async fn handler_error_does_not_stop_subsequent_tasks() -> anyhow::Result<()> {
        let processed = Arc::new(AsyncMutex::new(Vec::new()));
        let processed_for_worker = Arc::clone(&processed);

        let queue = LocalTaskQueue::connect(1, move |task: TaskEnvelope<TestTask>| {
            let processed = Arc::clone(&processed_for_worker);
            async move {
                let id = task.task.as_ref().unwrap().id;
                if id == 2 {
                    anyhow::bail!("intentional failure for task 2")
                }

                processed.lock().await.push(id);
                Ok(())
            }
        });

        for id in [1, 2, 3] {
            queue.enqueue(envelope(id)).await?;
        }

        wait_until("tasks 1 and 3 to complete", || {
            processed
                .try_lock()
                .map(|p| p.contains(&1) && p.contains(&3))
                .unwrap_or(false)
        })
        .await;

        let done = processed.lock().await.clone();
        assert!(!done.contains(&2), "the failing task must not be recorded");
        Ok(())
    }

    /// A zero concurrency limit would deadlock the dispatcher, so it is clamped.
    #[tokio::test]
    async fn zero_concurrency_is_clamped_to_one() -> anyhow::Result<()> {
        let done = Arc::new(AtomicUsize::new(0));
        let done_for_worker = Arc::clone(&done);

        let queue = LocalTaskQueue::connect(0, move |_task: TaskEnvelope<TestTask>| {
            let done = Arc::clone(&done_for_worker);
            async move {
                done.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        queue.enqueue(envelope(1)).await?;

        wait_until("the task to run", || done.load(Ordering::SeqCst) == 1).await;
        Ok(())
    }
}
