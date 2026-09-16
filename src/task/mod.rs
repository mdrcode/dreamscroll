mod task;
pub use task::*;

// TaskMaster is the primary entry point for manipulating Task instances.
mod taskmaster;
pub use taskmaster::*;

// StatusTracker maintains canonical source of truth for task status.
mod status_code;
pub use status_code::*;
mod status_tracker;
pub use status_tracker::*;

// StatusListener is the future LISTEN/NOTIFY thread that relays status to SSE.
mod status_listener;
pub use status_listener::*;

mod maker;
pub use maker::*;

mod taskqueue;
pub use taskqueue::*;
mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;
mod taskqueue_local;
pub use taskqueue_local::*;

use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskQueueBackend {
    Local,
    GCloudTasks,
}
