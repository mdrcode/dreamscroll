use serde::Serialize;

#[async_trait::async_trait]
pub trait TopicQueue: std::fmt::Debug + Send + Sync {
    type Payload: Serialize + Send;

    async fn enqueue(&self, payload: Self::Payload) -> anyhow::Result<()>;
}
