#[async_trait::async_trait]
pub trait TopicQueue: dyn_clone::DynClone + Send + Sync {
    async fn enqueue(&self, capture_id: i32) -> anyhow::Result<()>;
}

dyn_clone::clone_trait_object!(TopicQueue);
