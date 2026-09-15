use serde::Serialize;

/// A serializable definition for a unit of work.
pub trait Task: std::fmt::Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str;
}

/// A handle to a task once its payload has been submitted to a TaskQueue.
#[derive(Debug, Clone, Serialize)]
pub struct TaskWrapper<P: Task> {
    pub user_id: i32,
    pub task_id: String,
    pub payload: Option<P>, // convenience, but not always available (e.g. when dequeued)
}
