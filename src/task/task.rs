use serde::Serialize;

/// A serializable specification for a unit of work.
pub trait Task: std::fmt::Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str;
}

/// A proper Task, constructed from a Payload, once it has been submitted to a TaskQueue.
#[derive(Debug, Clone, Serialize)]
pub struct TaskEnvelope<T: Task> {
    pub user_id: i32,
    pub task_id: String,
    pub task: Option<T>, // convenience, but not always available (e.g. when dequeued)
}
