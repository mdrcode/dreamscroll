use std::{marker::PhantomData, sync::Arc};

use anyhow::anyhow;
use google_cloud_gax::conn::Environment;
use google_cloud_googleapis::pubsub::v1::PubsubMessage;
use google_cloud_pubsub::{
    client::{Client, ClientConfig},
    publisher::Publisher,
};
use serde::Serialize;

use super::*;

#[derive(Clone)]
pub struct PubSubTaskQueue<TTask> {
    inner: Arc<PubSubTaskQueueInner>,
    _task: PhantomData<TTask>,
}

struct PubSubTaskQueueInner {
    project_id: String,
    topic_id: String,
    emulator_host: Option<String>,
    publisher: Publisher,
}

impl<TTask> std::fmt::Debug for PubSubTaskQueue<TTask> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSubTaskQueue")
            .field("project_id", &self.inner.project_id)
            .field("topic_id", &self.inner.topic_id)
            .field("emulator_host", &self.inner.emulator_host)
            .finish()
    }
}

impl<TTask> PubSubTaskQueue<TTask> {
    pub async fn connect(
        project_id: &str,
        topic_id: &str,
        emulator_endpoint: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut config = ClientConfig::default();
        config.project_id = Some(project_id.to_string());

        let emulator_host = emulator_endpoint.map(util::trim_protocol_and_slash);

        if let Some(host) = emulator_host.as_deref() {
            config.environment = Environment::Emulator(host.to_string());
            tracing::info!(emulator_host = ?host, "Using Pub/Sub emulator");
        } else {
            config = config
                .with_auth()
                .await
                .map_err(|err| anyhow!("Failed to initialize Google Pub/Sub auth: {}", err))?;
        }

        let environment_debug = format!("{:?}", config.environment);
        let client = Client::new(config).await.map_err(|err| {
            anyhow!(
                "Failed to connect Pub/Sub client (environment: {}): {}",
                environment_debug,
                err
            )
        })?;

        let topic = client.topic(topic_id);
        let topic_exists = topic
            .exists(None)
            .await
            .map_err(|err| anyhow!("Failed checking Pub/Sub topic {}: {}", topic_id, err))?;

        if !topic_exists {
            return Err(anyhow!(
                "Configured Pub/Sub topic does not exist or is inaccessible: {}",
                topic_id
            ));
        }

        let publisher = topic.new_publisher(None);

        tracing::info!(
            project_id = ?project_id,
            topic_id = ?topic_id,
            emulator_host = ?emulator_host,
            "Configured Pub/Sub task queue"
        );

        Ok(Self {
            inner: Arc::new(PubSubTaskQueueInner {
                project_id: project_id.to_string(),
                topic_id: topic_id.to_string(),
                emulator_host,
                publisher,
            }),
            _task: PhantomData,
        })
    }
}

#[async_trait::async_trait]
impl<TTask: TaskId + Serialize + Send + Sync + 'static> TaskQueue for PubSubTaskQueue<TTask> {
    type Task = TTask;

    async fn enqueue(&self, task: TTask) -> anyhow::Result<()> {
        let task_json = serde_json::to_vec(&task)?;

        let awaiter = self
            .inner
            .publisher
            .publish(PubsubMessage {
                data: task_json.into(),
                ..Default::default()
            })
            .await;

        let message_id = awaiter
            .get()
            .await
            .map_err(|err| anyhow!("Pub/Sub publish failed: {}", err))?;

        tracing::debug!(message_id, "Published task to Pub/Sub");
        Ok(())
    }

    async fn get_status(&self, task_id: &str) -> anyhow::Result<TaskStatus> {
        unimplemented!();
    }
}

