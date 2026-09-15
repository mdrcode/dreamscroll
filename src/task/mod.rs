mod task;
pub use task::*;

// TaskId generation (placeholder — see task_id.rs).
mod task_id;
pub use task_id::*;

// TaskMaster is the primary entry point for manipulating Task instances.
mod taskmaster;
pub use taskmaster::*;

// TaskWatcher is the future LISTEN/NOTIFY thread that relays status to SSE.
mod taskwatcher;
pub use taskwatcher::*;

// TaskStatusRecorder owns the task_status table persistence.
mod status_recorder;
pub use status_recorder::*;

mod maker;
pub use maker::*;

mod taskqueue;
pub use taskqueue::*;

mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;

mod taskqueue_local;
pub use taskqueue_local::*;

mod taskqueue_pubsub;
pub use taskqueue_pubsub::*;

use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskQueueBackend {
    Local,
    GCloudPubSub,
    GCloudTasks,
}
