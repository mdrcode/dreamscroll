use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

use super::*;

#[derive(Clone)]
pub struct PubSubBaseUrl {
    publish_base_url: String,
}

impl PubSubBaseUrl {
    pub fn new(project_id: &str, emulator_url_base: Option<&str>) -> Self {
        let url_base = emulator_url_base
            .as_deref()
            .unwrap_or("https://pubsub.googleapis.com");

        Self {
            publish_base_url: format!(
                "{}/v1/projects/{}/topics", // note no trailing /
                url_base, project_id
            ),
        }
    }
}

#[derive(Clone)]
pub struct PubSubTopicQueue {
    publish_url: String,
    client: reqwest::Client,
}

impl PubSubTopicQueue {
    pub fn new(base: &PubSubBaseUrl, topic_id: &str) -> Self {
        let publish_url = format!("{}/{}:publish", base.publish_base_url, topic_id);

        tracing::info!(publish_url, "Configured Pub/Sub HTTP topic queue");

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
impl TopicQueue for PubSubTopicQueue {
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
