use std::{marker::PhantomData, sync::Arc};

use anyhow::{Context, anyhow};
use google_cloud_tasks_v2::client::CloudTasks;
use google_cloud_tasks_v2::model::{HttpMethod, HttpRequest, Task};
use serde::Serialize;

use super::*;

#[derive(Clone)]
pub struct FirestoreTaskQueue<TTask> {
    inner: Arc<FirestoreTaskQueueInner>,
    _task: PhantomData<TTask>,
}

#[derive(Debug)]
struct FirestoreTaskQueueInner {
    project_id: String,
    queue_path: String,
    task_webhook_url: String,
    emulator_host: Option<String>,
    client: CloudTasks,
}

impl<TTask> std::fmt::Debug for FirestoreTaskQueue<TTask> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirestoreTaskQueue")
            .field("project_id", &self.inner.project_id)
            .field("queue_path", &self.inner.queue_path)
            .field("task_webhook_url", &self.inner.task_webhook_url)
            .field("emulator_host", &self.inner.emulator_host)
            .finish()
    }
}

impl<TTask> FirestoreTaskQueue<TTask> {
    pub async fn connect(
        project_id: &str,
        region: &str,
        queue_id: &str,
        task_webhook_url: &str,
        emulator_endpoint: Option<&str>,
    ) -> anyhow::Result<Self> {
        let emulator_host = emulator_endpoint.map(util::trim_protocol_and_slash);

        let queue_path = format!(
            "projects/{}/locations/{}/queues/{}",
            project_id, region, queue_id
        );

        let client = {
            let mut builder = CloudTasks::builder();
            if let Some(emulator) = emulator_host.as_deref() {
                builder = builder.with_endpoint(emulator);
            }
            builder.build().await?
        };

        Ok(Self {
            inner: Arc::new(FirestoreTaskQueueInner {
                project_id: project_id.to_string(),
                queue_path,
                task_webhook_url: task_webhook_url.to_string(),
                emulator_host,
                client,
            }),
            _task: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<TTask: TaskId + Serialize + Send + Sync + 'static> TaskQueue for FirestoreTaskQueue<TTask> {
    type Task = TTask;

    async fn enqueue(&self, task: TTask) -> anyhow::Result<()> {
        let task_id = task.id();
        let body = serde_json::to_vec(&task).context("Failed to serialize task payload to JSON")?;

        let task_name = format!("{}/tasks/{}", self.inner.queue_path, task_id);

        let webhook_request = HttpRequest::new()
            .set_url(&self.inner.task_webhook_url)
            .set_http_method(HttpMethod::Post)
            .set_headers([("Content-Type", "application/json")])
            .set_body(body);

        let pending_task = Task::new()
            .set_name(task_name.clone())
            .set_http_request(webhook_request);

        let created_task = self
            .inner
            .client
            .create_task()
            .set_parent(self.inner.queue_path.clone())
            .set_task(pending_task)
            .send()
            .await
            .map_err(|err| anyhow!("Cloud Tasks create_task failed for {}: {}", task_name, err))?;

        tracing::debug!(
            queue = %self.inner.queue_path,
            task_name = %created_task.name,
            "Enqueued task to Cloud Tasks"
        );

        Ok(())
    }

    async fn get_status(&self, _task_id: &str) -> anyhow::Result<TaskStatus> {
        unimplemented!();
    }
}
