use serde::Serialize;

#[async_trait::async_trait]
pub trait TopicQueue: std::fmt::Debug + dyn_clone::DynClone + Send + Sync {
    async fn enqueue_payload(&self, payload_json: Vec<u8>) -> anyhow::Result<()>;
}

dyn_clone::clone_trait_object!(TopicQueue);

pub async fn enqueue_serializable<T: Serialize + ?Sized>(
    queue: &dyn TopicQueue,
    payload: &T,
) -> anyhow::Result<()> {
    let payload_json = serde_json::to_vec(payload)?;
    queue.enqueue_payload(payload_json).await
}
