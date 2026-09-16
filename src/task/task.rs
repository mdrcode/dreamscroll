use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A Task is a serializable specification of a unit of work.
pub trait Task: Clone + Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str;
    fn entity_type() -> &'static str;
    fn entity_id(&self) -> i32;
}

/// A TaskEnvelope is a proper wrapped Task once it has been submitted to a TaskQueue.
/// When querying task status after submission, it's likely that the full Task
/// definition is not available, so the caller should rely on the identity fields
/// within the envelope.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskEnvelope<T: Task> {
    pub user_id: i32,
    pub envelope_id: String,
    pub task: Option<T>, // convenience, not always available (e.g. when dequeued)
}

impl<T: Task> TaskEnvelope<T> {
    pub fn from_task(user_id: i32, task: T) -> Self {
        Self {
            user_id,
            envelope_id: format!(
                "u{}-{}-{}{}",
                user_id,
                T::task_type(),
                T::entity_type(),
                task.entity_id()
            ),
            task: Some(task),
        }
    }
}

impl<T: Task> std::fmt::Debug for TaskEnvelope<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("TaskEnvelope");
        debug.field("user_id", &self.user_id);
        debug.field("task_type", &T::task_type());
        debug.field("envelope_id", &self.envelope_id);

        // Show a bounded preview of the serialized payload so logs stay readable
        // even for large tasks (e.g. a spark task with many capture_ids).
        let payload_preview = self
            .task
            .as_ref()
            .map(|task| {
                let json =
                    serde_json::to_string(task).unwrap_or("<serialization error>".to_string());
                if json.chars().count() > 200 {
                    // Take a bounded char-boundary-safe prefix so we never panic
                    // on a multi-byte UTF-8 boundary.
                    let preview: String = json.chars().take(200).collect();
                    format!("{preview}...")
                } else {
                    json
                }
            })
            .unwrap_or("<no payload>".to_string());
        debug.field("payload", &payload_preview);

        debug.finish()
    }
}
