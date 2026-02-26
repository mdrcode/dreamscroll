use std::sync::Arc;

use anyhow::{Context, anyhow};
use google_cloud_gax::conn::Environment;
use google_cloud_googleapis::pubsub::v1::PubsubMessage;
use google_cloud_pubsub::{
    client::{Client, ClientConfig},
    publisher::Publisher,
};
use tokio::sync::OnceCell;

use crate::{facility, pubsub};

#[derive(Clone)]
pub struct PubSubTopicQueue {
    inner: Arc<PubSubTopicQueueInner>,
}

struct PubSubTopicQueueInner {
    project_id: String,
    topic_id: String,
    topic_fqn: String,
    publish_url: String,
    emulator_host: Option<String>,
    publisher: OnceCell<Publisher>,
}

impl std::fmt::Debug for PubSubTopicQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSubTopicQueue")
            .field("topic_fqn", &self.inner.topic_fqn)
            .field("publish_url", &self.inner.publish_url)
            .field("emulator_host", &self.inner.emulator_host)
            .finish()
    }
}

impl PubSubTopicQueue {
    pub fn new(project_id: &str, topic_id: &str, emulator_url_base: Option<&str>) -> Self {
        let topic_fqn = format!("projects/{}/topics/{}", project_id, topic_id);
        let publish_url = format!(
            "{}/v1/{}:publish",
            emulator_url_base.unwrap_or("https://pubsub.googleapis.com"),
            topic_fqn
        );
        let emulator_host = emulator_url_base.map(emulator_host_from_base_url);

        tracing::info!(
            topic_fqn,
            publish_url,
            emulator_host = ?emulator_host,
            "Configured Pub/Sub topic queue"
        );

        Self {
            inner: Arc::new(PubSubTopicQueueInner {
                project_id: project_id.to_string(),
                topic_id: topic_id.to_string(),
                topic_fqn,
                publish_url,
                emulator_host,
                publisher: OnceCell::new(),
            }),
        }
    }

    pub fn from_config(config: &facility::Config) -> Self {
        Self::new(
            config.gcloud_project_id.as_str(),
            config.pubsub_topic_id_new_capture.as_str(),
            config.pubsub_emulator_base_url.as_deref(),
        )
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
impl pubsub::TopicQueue for PubSubTopicQueue {
    async fn enqueue_payload(&self, payload_json: Vec<u8>) -> anyhow::Result<()> {
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
