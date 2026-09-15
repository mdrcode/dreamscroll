use serde::Serialize;

pub trait TaskId {
    fn id(&self) -> String;
}

#[async_trait::async_trait]
pub trait TaskQueue: std::fmt::Debug + Send + Sync {
    type Task: Send + Serialize + TaskId;

    async fn enqueue(&self, task: Self::Task) -> anyhow::Result<()>;
}
