use super::*;

#[async_trait::async_trait]
pub trait TaskQueue<Payload: TaskPayload>: std::fmt::Debug + Send + Sync {
    async fn enqueue(&self, task: Payload) -> anyhow::Result<()>;
}
