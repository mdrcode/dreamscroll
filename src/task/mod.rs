// Task and TaskEnvelope, the core concepts
mod task_def;
pub use task_def::*;

// TaskMaster is the primary entry point for manipulating Task instances.
mod taskmaster;
pub use taskmaster::*;

// TaskRunStatus models progress of a background Run (invocation) of a Task
mod taskrunstatus;
pub use taskrunstatus::*;

// TaskRunTracker supports querying the status of task runs, and (in the future)
// will support subscribing to status changes.
mod taskruntracker;
pub use taskruntracker::*;

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
