use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

use crate::{facility, task};

#[derive(Clone)]
pub struct PubSubHttpTaskQueue {
    client: reqwest::Client,
    publish_url: String,
    bearer_token: Option<String>,
}

impl PubSubHttpTaskQueue {
    pub fn from_config(config: &facility::Config) -> Self {
        let project_id = config
            .pubsub_project_id
            .as_ref()
            .expect("PubSubHttpTaskQueue creation fatal: DREAMSCROLL_PUBSUB_PROJECT_ID missing");

        let topic_id = config
            .pubsub_topic_id
            .as_ref()
            .expect("PubSubHttpTaskQueue creation fatal: DREAMSCROLL_PUBSUB_TOPIC_ID missing");

        let base_url = config
            .pubsub_api_base_url
            .as_deref()
            .unwrap_or("https://pubsub.googleapis.com")
            .trim_end_matches('/');

        let publish_url = format!(
            "{}/v1/projects/{}/topics/{}:publish",
            base_url, project_id, topic_id
        );

        Self {
            client: reqwest::Client::new(),
            publish_url,
            bearer_token: config.pubsub_publish_bearer_token.clone(),
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
impl task::TaskQueue for PubSubHttpTaskQueue {
    async fn enqueue_illumination(&self, capture_id: i32) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(&TaskPayload { capture_id })?;
        let encoded = STANDARD.encode(payload);

        let body = PublishRequest {
            messages: vec![PublishMessage { data: encoded }],
        };

        let mut request = self.client.post(&self.publish_url).json(&body);
        if let Some(token) = &self.bearer_token {
            request = request.bearer_auth(token);
        }

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
