use serde::Serialize;

/// A serializable definition for a unit of work.
pub trait TaskPayload: std::fmt::Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str;
}

/// A handle to a task once its payload has been submitted to a TaskQueue.
#[derive(Debug, Clone, Serialize)]
pub struct TaskHandle<P: TaskPayload> {
    pub user_id: i32,
    pub id: String,
    pub payload: Option<P>, // convenience, but not always available (e.g. when dequeued)
}
