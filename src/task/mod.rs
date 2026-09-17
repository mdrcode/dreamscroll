// Task and TaskEnvelope, the core concepts
mod task_def;
pub use task_def::*;

// TaskStatus models the Status of a Task Run
mod taskstatus;
pub use taskstatus::*;

// TaskMaster is the primary entry point for manipulating Task instances.
mod taskmaster;
pub use taskmaster::*;

// TaskStatusTracker supports querying the status of task runs, and (in the future)
// will support subscribing to status changes.
mod taskstatustracker;
pub use taskstatustracker::*;

// TaskQueue trait which abstracts over the underlying queue backends.
mod taskqueue;
pub use taskqueue::*;
mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;
mod taskqueue_local;
pub use taskqueue_local::*;

mod maker;
pub use maker::*;

use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskQueueBackend {
    Local,
    GCloudTasks,
}
