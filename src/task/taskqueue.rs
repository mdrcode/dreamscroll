use super::*;

#[async_trait::async_trait]
pub trait TaskQueue<T: Task>: std::fmt::Debug + Send + Sync {
    async fn enqueue(&self, wrapped: TaskEnvelope<T>) -> anyhow::Result<()>;
}
