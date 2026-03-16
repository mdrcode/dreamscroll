/// A simple local-only HTTP client for invoking webhook endpoints directly.
/// Used by the LocalTaskQueue implementation for local development so tasks
/// actually execute as close to the real prod flow as possible.
/// No auth and assumes the server is running locally.
#[derive(Clone, Debug)]
pub struct DevWebhookClient {
    base_url: String,
    client: reqwest::Client,
}

impl DevWebhookClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn post_illuminate(
        &self,
        task: &super::logic::illuminate::IlluminationTask,
    ) -> anyhow::Result<()> {
        let url = format!("{}/_wh/cloudtask/illuminate", self.base_url);
        let response = self.client.post(&url).json(task).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("DevWebhookClient illuminate failed ({}): {}", status, body);
        }
        Ok(())
    }

    pub async fn post_spark(&self, task: &super::logic::spark::SparkTask) -> anyhow::Result<()> {
        let url = format!("{}/_wh/cloudtask/spark", self.base_url);
        let response = self.client.post(&url).json(task).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("DevWebhookClient spark failed ({}): {}", status, body);
        }
        Ok(())
    }
}
