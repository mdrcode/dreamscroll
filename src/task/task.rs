use serde::{Deserialize, Serialize};

/// A serializable specification for a unit of work.
pub trait Task: std::fmt::Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str;
}

/// A proper wrapped Task once it has been submitted to a TaskQueue.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskEnvelope<T: Task> {
    pub user_id: i32,
    pub task_id: String,
    pub task: Option<T>, // convenience, but not always available (e.g. when dequeued)
}

impl<T: Task> std::fmt::Debug for TaskEnvelope<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("TaskEnvelope");
        debug.field("task_type", &T::task_type());
        debug.field("user_id", &self.user_id);
        debug.field("task_id", &self.task_id);

        // Show a bounded preview of the serialized payload so logs stay readable
        // even for large tasks (e.g. a spark task with many capture_ids).
        let payload_preview = self
            .task
            .as_ref()
            .map(|task| {
                let json =
                    serde_json::to_string(task).unwrap_or("<serialization error>".to_string());
                if json.len() > 200 {
                    format!("{}...", &json[..200])
                } else {
                    json
                }
            })
            .unwrap_or("<no payload>".to_string());
        debug.field("payload", &payload_preview);

        debug.finish()
    }
}
