#[async_trait::async_trait]
pub trait TopicQueue: std::fmt::Debug + dyn_clone::DynClone + Send + Sync {
    async fn enqueue(&self, capture_id: i32) -> anyhow::Result<()>;
}

dyn_clone::clone_trait_object!(TopicQueue);
