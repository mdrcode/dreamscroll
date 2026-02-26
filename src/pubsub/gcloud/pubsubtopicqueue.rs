use std::{marker::PhantomData, sync::Arc};

use anyhow::anyhow;
use google_cloud_gax::conn::Environment;
use google_cloud_googleapis::pubsub::v1::PubsubMessage;
use google_cloud_pubsub::{
    client::{Client, ClientConfig},
    publisher::Publisher,
};
use serde::Serialize;

use crate::{facility, pubsub};

#[derive(Clone)]
pub struct PubSubTopicQueue<TPayload> {
    inner: Arc<PubSubTopicQueueInner>,
    _payload: PhantomData<TPayload>,
}

struct PubSubTopicQueueInner {
    topic_fqn: String,
    publish_url: String,
    emulator_host: Option<String>,
    publisher: Publisher,
}

impl<TPayload> std::fmt::Debug for PubSubTopicQueue<TPayload> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSubTopicQueue")
            .field("topic_fqn", &self.inner.topic_fqn)
            .field("publish_url", &self.inner.publish_url)
            .field("emulator_host", &self.inner.emulator_host)
            .finish()
    }
}

impl<TPayload> PubSubTopicQueue<TPayload> {
    pub async fn new(
        project_id: &str,
        topic_id: &str,
        emulator_url_base: Option<&str>,
    ) -> anyhow::Result<Self> {
        let topic_fqn = format!("projects/{}/topics/{}", project_id, topic_id);
        let publish_url = format!(
            "{}/v1/{}:publish",
            emulator_url_base.unwrap_or("https://pubsub.googleapis.com"),
            topic_fqn
        );
        let emulator_host = emulator_url_base.map(emulator_host_from_base_url);

        let mut config = ClientConfig::default();
        config.project_id = Some(project_id.to_string());

        if let Some(host) = &emulator_host {
            config.environment = Environment::Emulator(host.clone());
            tracing::info!(host, "Using Pub/Sub emulator");
        } else {
            config = config
                .with_auth()
                .await
                .map_err(|err| anyhow!("Failed to initialize Google Pub/Sub auth: {}", err))?;
        }

        let client = Client::new(config)
            .await
            .map_err(|err| anyhow!("Failed to create Pub/Sub client: {}", err))?;
        let publisher = client.topic(topic_id).new_publisher(None);

        tracing::info!(
            topic_fqn,
            publish_url,
            emulator_host = ?emulator_host,
            "Configured Pub/Sub topic queue"
        );

        Ok(Self {
            inner: Arc::new(PubSubTopicQueueInner {
                topic_fqn,
                publish_url,
                emulator_host,
                publisher,
            }),
            _payload: PhantomData,
        })
    }

    pub async fn from_config(config: &facility::Config) -> anyhow::Result<Self> {
        Self::new(
            config.gcloud_project_id.as_str(),
            config.pubsub_topic_id_new_capture.as_str(),
            config.pubsub_emulator_base_url.as_deref(),
        )
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
impl<TPayload: Serialize + Send + Sync + 'static> pubsub::TopicQueue for PubSubTopicQueue<TPayload> {
    type Payload = TPayload;

    async fn enqueue(&self, payload: TPayload) -> anyhow::Result<()> {
        let payload_json = serde_json::to_vec(&payload)?;

        let awaiter = self
            .inner
            .publisher
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
