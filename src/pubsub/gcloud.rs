use std::{marker::PhantomData, sync::Arc};

use anyhow::{Context, anyhow};
use google_cloud_gax::conn::Environment;
use google_cloud_googleapis::pubsub::v1::PubsubMessage;
use google_cloud_pubsub::{
    client::{Client, ClientConfig},
    publisher::Publisher,
};
use serde::Serialize;
use tokio::sync::OnceCell;

use super::*;

#[derive(Clone)]
pub struct PubSubTopicQueue<TPayload> {
    inner: Arc<PubSubTopicQueueInner>,
    _payload: PhantomData<TPayload>,
}

struct PubSubTopicQueueInner {
    emulator_host: Option<String>,
    project_id: String,
    topic_id: String,
    publisher: OnceCell<Publisher>,
}

impl<TPayload> std::fmt::Debug for PubSubTopicQueue<TPayload> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSubTopicQueue")
            .field("project_id", &self.inner.project_id)
            .field("topic_id", &self.inner.topic_id)
            .field("emulator_host", &self.inner.emulator_host)
            .finish()
    }
}

impl<TPayload> PubSubTopicQueue<TPayload> {
    pub fn new(emulator_url_base: Option<&str>, project_id: &str, topic_id: &str) -> Self {
        let emulator_host = emulator_url_base.map(emulator_host_from_base_url);

        tracing::info!(
            project_id = ?project_id,
            topic_id = ?topic_id,
            emulator_host = ?emulator_host,
            "Configured Pub/Sub topic queue"
        );

        Self {
            inner: Arc::new(PubSubTopicQueueInner {
                emulator_host,
                project_id: project_id.to_string(),
                topic_id: topic_id.to_string(),
                publisher: OnceCell::new(),
            }),
            _payload: PhantomData,
        }
    }

    async fn publisher(&self) -> anyhow::Result<&Publisher> {
        let project_id = self.inner.project_id.clone();
        let topic_id = self.inner.topic_id.clone();
        let emulator_host = self.inner.emulator_host.clone();

        self.inner
            .publisher
            .get_or_try_init(move || async move {
                let mut config = ClientConfig::default();
                config.project_id = Some(project_id.clone());

                if let Some(host) = emulator_host {
                    config.environment = Environment::Emulator(host.clone());
                    tracing::info!(host, "Using Pub/Sub emulator");
                } else {
                    config = config
                        .with_auth()
                        .await
                        .context("Failed to initialize Google Pub/Sub auth")?;
                }

                let client = Client::new(config)
                    .await
                    .map_err(|err| anyhow!("Failed to create Pub/Sub client: {}", err))?;

                Ok(client.topic(&topic_id).new_publisher(None))
            })
            .await
    }
}

fn emulator_host_from_base_url(url_base: &str) -> String {
    let trimmed = url_base.trim().trim_end_matches('/');
    trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed)
        .to_string()
}

#[async_trait::async_trait]
impl<TPayload: Serialize + Send + Sync + 'static> TopicQueue for PubSubTopicQueue<TPayload> {
    type Payload = TPayload;

    async fn enqueue(&self, payload: TPayload) -> anyhow::Result<()> {
        let payload_json = serde_json::to_vec(&payload)?;
        let publisher = self.publisher().await?;

        let awaiter = publisher
            .publish(PubsubMessage {
                data: payload_json.into(),
                ..Default::default()
            })
            .await;

        let message_id = awaiter
            .get()
            .await
            .map_err(|err| anyhow!("Pub/Sub publish failed: {}", err))?;

        tracing::debug!(message_id, "Published payload to Pub/Sub");
        Ok(())
    }
}
