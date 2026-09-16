use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A Task is a serializable specification of a unit of work.
pub trait Task: Clone + Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str; // eg. "illuminate" or "spark"
    fn entity_type() -> &'static str; // eg. "capture" or "spark"
    fn entity_id(&self) -> i32;

    // Any greater "payload" (e.g. which model to use, which prompt, etc)
    // is up to the concrete task implementation to define and serialize.
}

/// A TaskEnvelope is a proper wrapped Task once it has been submitted to a TaskQueue.
/// When querying task status after submission, it's likely that the full Task
/// definition is not available, so the caller should rely on the identity fields
/// within the envelope.
///
/// `envelope_id` identifies the *logical* task; `run` identifies one attempt to
/// carry it out. Together they key a `task_status` row, so a rerun of settled
/// work is a new run rather than an overwrite.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskEnvelope<T: Task> {
    pub user_id: i32,
    pub envelope_id: String,
    /// Which run of this logical task this envelope carries, counting from 1.
    #[serde(default = "first_run")]
    pub run: i32,
    pub task: Option<T>, // convenience, not always available (e.g. when dequeued)
}

fn first_run() -> i32 {
    1
}

impl<T: Task> TaskEnvelope<T> {
    /// Build an envelope for a run of a task.
    ///
    /// `run` counts from 1; callers get it from the latest `task_status` row.
    pub fn new(user_id: i32, task: T, run: i32) -> Self {
        Self {
            user_id,
            envelope_id: Self::make_envelope_id(user_id, &task),
            run,
            task: Some(task),
        }
    }

    /// Build the deterministic identity for a logical task, e.g.
    /// `u1-illuminate-capture123`. Encodes user_id + task_type + entity.
    ///
    /// Deliberately *excludes* the run: the id names the work, not one attempt
    /// at it. It is a static so callers can compute the id before an envelope
    /// exists, which is what lets a submitter look up the latest run first.
    pub fn make_envelope_id(user_id: i32, task: &T) -> String {
        format!(
            "u{}-{}-{}{}",
            user_id,
            T::task_type(),
            T::entity_type(),
            task.entity_id()
        )
    }
}

impl<T: Task> std::fmt::Debug for TaskEnvelope<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("TaskEnvelope");
        debug.field("task_type", &T::task_type());
        debug.field("user_id", &self.user_id);
        debug.field("envelope_id", &self.envelope_id);
        debug.field("run", &self.run);

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
