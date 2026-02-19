use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

use crate::{facility, task};

#[derive(Clone)]
pub struct PubSubHttpBaseUrl {
    publish_base_url: String,
}

impl PubSubHttpBaseUrl {
    pub fn from_config(config: &facility::DreamscrollPubSubConfig) -> Self {
        Self {
            publish_base_url: format!(
                "{}/v1/projects/{}/topics", // note no trailing /
                config.api_base_url, config.project_id
            ),
        }
    }
}

#[derive(Clone)]
pub struct PubSubHttpTaskQueue {
    publish_url: String,
    client: reqwest::Client,
}

impl PubSubHttpTaskQueue {
    pub fn new(base: PubSubHttpBaseUrl, topic_id: &str) -> Self {
        let publish_url = format!("{}/{}:publish", base.publish_base_url, topic_id);

        tracing::info!(publish_url, "Configured Pub/Sub HTTP task queue");

        Self {
            client: reqwest::Client::new(),
            publish_url,
        }
    }
}

#[derive(Serialize)]
struct TaskPayload {
    capture_id: i32,
}

#[derive(Serialize)]
struct PublishMessage {
    data: String,
}

#[derive(Serialize)]
struct PublishRequest {
    messages: Vec<PublishMessage>,
}

#[async_trait::async_trait]
impl task::TopicQueue for PubSubHttpTaskQueue {
    async fn enqueue(&self, capture_id: i32) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(&TaskPayload { capture_id })?;
        let encoded = STANDARD.encode(payload);

        let body = PublishRequest {
            messages: vec![PublishMessage { data: encoded }],
        };

        let request = self.client.post(&self.publish_url).json(&body);

        // if let Some(token) = &self.bearer_token {
        //     request = request.bearer_auth(token);
        // }

        let response = request.send().await?;
        let status = response.status();

        if !status.is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<unable to read error body>".to_string());
            return Err(anyhow!(
                "Pub/Sub publish failed with status {}: {}",
                status,
                text
            ));
        }

        tracing::debug!(capture_id, "Published illumination task to Pub/Sub");
        Ok(())
    }
}
