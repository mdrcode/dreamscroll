/// A simple local-only HTTP client for invoking webhook endpoints directly.
/// Used by the LocalTaskQueue implementation for local development so tasks
/// actually execute as close to the real prod flow as possible.
/// No auth and assumes the server is running locally.
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct LocalWebhookClient {
    base_url: String,
    client: reqwest::Client,
}

impl LocalWebhookClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn post_task<T: Serialize + ?Sized>(
        &self,
        webhook_local_path: &str,
        task: &T,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            webhook_local_path.trim_start_matches('/')
        );
        let response = self.client.post(&url).json(task).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "LocalWebhookClient post failed path='{}' ({}): {}",
                webhook_local_path,
                status,
                body
            );
        }
        tracing::debug!(url, "LocalWebhookClient post successful");
        Ok(())
    }
}
