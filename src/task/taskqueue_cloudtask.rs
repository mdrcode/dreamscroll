use std::{marker::PhantomData, sync::Arc};

use anyhow::{Context, anyhow};
use google_cloud_tasks_v2::client::CloudTasks;
use google_cloud_tasks_v2::model::{HttpMethod, HttpRequest, OidcToken, Task as CloudTask};

use super::*;

pub fn make_cloud_tasks_task_name(queue_path: &str, envelope: &TaskEnvelope<impl Task>) -> String {
    format!(
        "{queue_path}/tasks/{}-run{}",
        envelope.envelope_id, envelope.run
    )
}

/// Dispatches submitted tasks to a Google Cloud Tasks queue, which will call
/// the configured webhook URL with an OIDC token.
#[derive(Clone)]
pub struct CloudTaskQueue<T: Task> {
    inner: Arc<CloudTaskQueueInner>,
    _task: PhantomData<T>,
}
#[derive(Debug)]
struct CloudTaskQueueInner {
    client: CloudTasks,
    cloud_tasks_queue_path: String,
    task_webhook_url: String,
    oidc_token: OidcToken,
}

impl<T: Task> std::fmt::Debug for CloudTaskQueue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudTaskQueue")
            .field("cloud_tasks_queue_path", &self.inner.cloud_tasks_queue_path)
            .finish()
    }
}

impl<T: Task> CloudTaskQueue<T> {
    pub async fn connect(
        cloud_tasks_queue_path: String,
        webhook_url: String,
        oidc_token: OidcToken,
    ) -> anyhow::Result<Self> {
        let client = CloudTasks::builder().build().await?;

        Ok(Self {
            inner: Arc::new(CloudTaskQueueInner {
                cloud_tasks_queue_path,
                client,
                task_webhook_url: webhook_url,
                oidc_token,
            }),
            _task: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<T: Task + 'static> TaskQueue<T> for CloudTaskQueue<T> {
    async fn enqueue(&self, envelope: TaskEnvelope<T>) -> anyhow::Result<()> {
        // Serialize the full envelope (task definition + identity) so the worker knows
        // which task and run it's completing.
        let body =
            serde_json::to_vec(&envelope).context("Failed to serialize task wrapper to JSON")?;

        let webhook_request = HttpRequest::new()
            .set_url(self.inner.task_webhook_url.clone())
            .set_http_method(HttpMethod::Post)
            .set_headers([("Content-Type", "application/json")])
            .set_oidc_token(self.inner.oidc_token.clone())
            .set_body(body);

        // A deterministic name makes client retries idempotent at the Cloud
        // Tasks layer. The run is included so intentional reruns get a new
        // Cloud Task name.
        let pending_task = CloudTask::new()
            .set_name(make_cloud_tasks_task_name(
                &self.inner.cloud_tasks_queue_path,
                &envelope,
            ))
            .set_http_request(webhook_request);

        let created_task = self
            .inner
            .client
            .create_task()
            .set_parent(self.inner.cloud_tasks_queue_path.clone())
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
            queue = %self.inner.cloud_tasks_queue_path,
            task_name = %created_task.name,
            envelope = ?envelope,
            "Enqueued task envelope to queue: {} with task_name: {}",
            self.inner.cloud_tasks_queue_path,
            created_task.name
        );

        Ok(())
    }
}
