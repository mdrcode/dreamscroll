use std::{
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use serde::Serialize;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

use super::*;

type TaskHandlerFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'static>>;
type TaskHandler<TTask> = Arc<dyn Fn(TTask) -> TaskHandlerFuture + Send + Sync + 'static>;

pub struct LocalTaskQueue<TTask> {
    inner: Arc<LocalTaskQueueInner<TTask>>,
    _task: PhantomData<TTask>,
}

struct LocalTaskQueueInner<TTask> {
    task_sender: mpsc::UnboundedSender<TTask>,
    max_concurrent_tasks: usize,
    dispatcher_handle: Mutex<Option<JoinHandle<()>>>,
}

impl<TTask> Clone for LocalTaskQueue<TTask> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _task: PhantomData,
        }
    }
}

impl<TTask> fmt::Debug for LocalTaskQueue<TTask> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTaskQueue")
            .field("max_concurrent_tasks", &self.inner.max_concurrent_tasks)
            .finish()
    }
}

impl<TTask> LocalTaskQueue<TTask>
where
    TTask: TaskId + Serialize + Send + Sync + 'static,
{
    pub fn connect<F, Fut>(max_concurrent_tasks: usize, task_handler: F) -> Self
    where
        F: Fn(TTask) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let max_concurrent_tasks = max_concurrent_tasks.max(1);

        let handler: TaskHandler<TTask> =
            Arc::new(move |task: TTask| -> TaskHandlerFuture { Box::pin(task_handler(task)) });
        let semaphore = Arc::new(Semaphore::new(max_concurrent_tasks));
        let (task_sender, mut task_receiver) = mpsc::unbounded_channel::<TTask>();

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
        task_receiver: &mut mpsc::UnboundedReceiver<TTask>,
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

impl<TTask> Drop for LocalTaskQueue<TTask> {
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
impl<TTask> TaskQueue for LocalTaskQueue<TTask>
where
    TTask: TaskId + Serialize + Send + Sync + 'static,
{
    type Task = TTask;

    async fn enqueue(&self, task: Self::Task) -> anyhow::Result<()> {
        self.inner
            .task_sender
            .send(task)
            .map_err(|_| anyhow!("Cannot enqueue into LocalTaskQueue after shutdown"))
    }

    async fn get_status(&self, _task_id: &str) -> anyhow::Result<TaskStatus> {
        unimplemented!();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::sync::{Mutex as AsyncMutex, oneshot};

    use super::*;

    #[derive(Debug, Clone, serde::Serialize)]
    struct TestTask {
        id: i32,
    }

    impl TaskId for TestTask {
        fn id(&self) -> String {
            self.id.to_string()
        }
    }

    #[tokio::test]
    async fn local_queue_executes_enqueued_tasks() -> anyhow::Result<()> {
        let seen = Arc::new(AsyncMutex::new(Vec::new()));
        let seen_for_worker = Arc::clone(&seen);

        let queue = LocalTaskQueue::connect(4, move |task: TestTask| {
            let seen = Arc::clone(&seen_for_worker);
            async move {
                seen.lock().await.push(task.id);
                Ok(())
            }
        });

        queue.enqueue(TestTask { id: 1 }).await?;
        queue.enqueue(TestTask { id: 2 }).await?;

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if seen.lock().await.len() == 2 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;

        let values = seen.lock().await.clone();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&1));
        assert!(values.contains(&2));
        Ok(())
    }

    #[tokio::test]
    async fn local_queue_processes_all_tasks_with_parallel_execution() -> anyhow::Result<()> {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));

        let active_for_worker = Arc::clone(&active);
        let max_active_for_worker = Arc::clone(&max_active);
        let done_for_worker = Arc::clone(&done);

        let queue = LocalTaskQueue::connect(3, move |_task: TestTask| {
            let active = Arc::clone(&active_for_worker);
            let max_active = Arc::clone(&max_active_for_worker);
            let done = Arc::clone(&done_for_worker);

            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                let mut prev = max_active.load(Ordering::SeqCst);
                while current > prev {
                    match max_active.compare_exchange(
                        prev,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(actual_prev) => prev = actual_prev,
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                done.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        for id in 0..6 {
            queue.enqueue(TestTask { id }).await?;
        }

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while done.load(Ordering::SeqCst) < 6 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;

        assert!(
            max_active.load(Ordering::SeqCst) >= 2,
            "expected at least two tasks to run in parallel"
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_queue_shutdown_does_not_drain_remaining_queue() -> anyhow::Result<()> {
        let (first_task_started_tx, first_task_started_rx) = oneshot::channel::<()>();
        let first_task_started_tx = Arc::new(AsyncMutex::new(Some(first_task_started_tx)));

        let (release_first_task_tx, release_first_task_rx) = oneshot::channel::<()>();
        let release_first_task_rx = Arc::new(AsyncMutex::new(Some(release_first_task_rx)));

        let processed = Arc::new(AtomicUsize::new(0));
        let processed_for_worker = Arc::clone(&processed);
        let first_task_started_tx_for_worker = Arc::clone(&first_task_started_tx);
        let release_first_task_rx_for_worker = Arc::clone(&release_first_task_rx);

        let queue = LocalTaskQueue::connect(1, move |_task: TestTask| {
            let processed = Arc::clone(&processed_for_worker);
            let first_task_started_tx = Arc::clone(&first_task_started_tx_for_worker);
            let release_first_task_rx = Arc::clone(&release_first_task_rx_for_worker);

            async move {
                let mut maybe_started_tx = first_task_started_tx.lock().await;
                if let Some(tx) = maybe_started_tx.take() {
                    let _ = tx.send(());

                    let mut maybe_release_rx = release_first_task_rx.lock().await;
                    if let Some(rx) = maybe_release_rx.take() {
                        let _ = rx.await;
                    }
                }
                drop(maybe_started_tx);

                processed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        queue.enqueue(TestTask { id: 1 }).await?;

        first_task_started_rx.await?;

        queue.enqueue(TestTask { id: 2 }).await?;
        queue.enqueue(TestTask { id: 3 }).await?;

        drop(queue);

        let _ = release_first_task_tx.send(());

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if processed.load(Ordering::SeqCst) >= 1 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        assert_eq!(
            processed.load(Ordering::SeqCst),
            1,
            "queued tasks should not be drained after shutdown"
        );
        Ok(())
    }

    #[tokio::test]
    async fn local_queue_handler_error_does_not_stop_subsequent_tasks() -> anyhow::Result<()> {
        let processed = Arc::new(AsyncMutex::new(Vec::new()));
        let processed_for_worker = Arc::clone(&processed);

        let queue = LocalTaskQueue::connect(1, move |task: TestTask| {
            let processed = Arc::clone(&processed_for_worker);
            async move {
                if task.id == 2 {
                    anyhow::bail!("intentional failure for task 2")
                }

                processed.lock().await.push(task.id);
                Ok(())
            }
        });

        queue.enqueue(TestTask { id: 1 }).await?;
        queue.enqueue(TestTask { id: 2 }).await?;
        queue.enqueue(TestTask { id: 3 }).await?;

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let done = processed.lock().await.clone();
                if done.contains(&1) && done.contains(&3) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await?;

        let done = processed.lock().await.clone();
        assert!(done.contains(&1), "task 1 should have completed");
        assert!(done.contains(&3), "task 3 should have completed");
        assert!(
            !done.contains(&2),
            "task 2 should fail and not be marked as completed"
        );
        Ok(())
    }
}
