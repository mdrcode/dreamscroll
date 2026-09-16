use std::{marker::PhantomData, sync::Arc};

use anyhow::{Context, anyhow};
use google_cloud_tasks_v2::client::CloudTasks;
use google_cloud_tasks_v2::model::{HttpMethod, HttpRequest, Task as CloudTask};

use super::*;

#[derive(Clone)]
pub struct CloudTaskQueue<T: Task> {
    inner: Arc<CloudTaskQueueInner>,
    _task: PhantomData<T>,
}

#[derive(Debug)]
struct CloudTaskQueueInner {
    queue_path: String,
    client: CloudTasks,
}

impl<T: Task> std::fmt::Debug for CloudTaskQueue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudTaskQueue")
            .field("queue_path", &self.inner.queue_path)
            .finish()
    }
}

impl<T: Task> CloudTaskQueue<T> {
    pub async fn connect(project_id: &str, region: &str, queue_id: &str) -> anyhow::Result<Self> {
        let client = CloudTasks::builder().build().await?;

        let queue_path = format!(
            "projects/{}/locations/{}/queues/{}",
            project_id, region, queue_id
        );

        Ok(Self {
            inner: Arc::new(CloudTaskQueueInner { queue_path, client }),
            _task: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<T: Task + 'static> TaskQueue<T> for CloudTaskQueue<T> {
    async fn enqueue(&self, envelope: TaskEnvelope<T>) -> anyhow::Result<()> {
        // Serialize the full wrapper (task identity + task) so the worker knows
        // which task it's completing.
        let body =
            serde_json::to_vec(&envelope).context("Failed to serialize task wrapper to JSON")?;

        let webhook_request = HttpRequest::new()
            .set_url("https://dummy-url-should-be-overridden-by-queue-config.dreamscroll.ai")
            .set_http_method(HttpMethod::Post)
            .set_headers([("Content-Type", "application/json")])
            .set_body(body);

        let pending_task = CloudTask::new().set_http_request(webhook_request);

        let created_task = self
            .inner
            .client
            .create_task()
            .set_parent(self.inner.queue_path.clone())
            .set_task(pending_task)
            .send()
            .await
            .map_err(|err| {
                anyhow!(
                    "Cloud Tasks create_task failed for task_id {:?}: {}",
                    envelope.envelope_id,
                    err
                )
            })?;

        tracing::info!(
            queue = %self.inner.queue_path,
            task_name = %created_task.name,
            "Enqueued task id {:?} to queue: {} with task_name: {}",
            envelope.envelope_id,
            self.inner.queue_path,
            created_task.name
        );

        Ok(())
    }
}
