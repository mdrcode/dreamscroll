use std::sync::Arc;

use anyhow::anyhow;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;

#[async_trait::async_trait]
pub trait IlluminationTaskPublisher: Send + Sync {
    async fn publish_capture_id(&self, capture_id: i32) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct NoopTaskPublisher;

#[async_trait::async_trait]
impl IlluminationTaskPublisher for NoopTaskPublisher {
    async fn publish_capture_id(&self, _capture_id: i32) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct PubSubHttpTaskPublisher {
    client: reqwest::Client,
    publish_url: String,
    bearer_token: Option<String>,
}

impl PubSubHttpTaskPublisher {
    pub fn new(publish_url: String, bearer_token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            publish_url,
            bearer_token,
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
impl IlluminationTaskPublisher for PubSubHttpTaskPublisher {
    async fn publish_capture_id(&self, capture_id: i32) -> anyhow::Result<()> {
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

pub fn make_task_publisher(config: &crate::facility::Config) -> Arc<dyn IlluminationTaskPublisher> {
    let Some(project_id) = &config.pubsub_project_id else {
        tracing::info!("Pub/Sub task publisher disabled (DREAMSCROLL_PUBSUB_PROJECT_ID missing)");
        return Arc::new(NoopTaskPublisher);
    };
    let Some(topic_id) = &config.pubsub_topic_id else {
        tracing::info!("Pub/Sub task publisher disabled (DREAMSCROLL_PUBSUB_TOPIC_ID missing)");
        return Arc::new(NoopTaskPublisher);
    };

    let base_url = config
        .pubsub_api_base_url
        .as_deref()
        .unwrap_or("https://pubsub.googleapis.com")
        .trim_end_matches('/');

    let publish_url = format!(
        "{}/v1/projects/{}/topics/{}:publish",
        base_url, project_id, topic_id
    );

    tracing::info!(publish_url, "Pub/Sub task publisher enabled");

    Arc::new(PubSubHttpTaskPublisher::new(
        publish_url,
        config.pubsub_publish_bearer_token.clone(),
    ))
}
