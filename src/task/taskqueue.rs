#[async_trait::async_trait]
pub trait TaskQueue: dyn_clone::DynClone + Send + Sync {
    async fn enqueue_illumination(&self, capture_id: i32) -> anyhow::Result<()>;
}

dyn_clone::clone_trait_object!(TaskQueue);
