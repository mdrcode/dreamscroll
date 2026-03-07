use serde::Serialize;

pub trait TaskId {
    fn id(&self) -> String;
}

#[async_trait::async_trait]
pub trait TaskQueue: std::fmt::Debug + Send + Sync {
    type Task: Send + Serialize + TaskId;

    async fn enqueue(&self, task: Self::Task) -> anyhow::Result<()>;

    async fn get_status(&self, task_id: &str) -> anyhow::Result<TaskStatus>;
}

pub enum Status {
    Queued,
    InProgress,
    Completed,
    Error,
    ErrorFinal,
}

pub struct TaskStatus {
    id: String,
    attempts: u32,
    status: Status,
}
