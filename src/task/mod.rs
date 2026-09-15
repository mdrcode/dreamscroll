// TaskMaster is the primary entry point for manipulating Task instances.
mod taskmaster;
pub use taskmaster::*;

// TaskWatcher is the future LISTEN/NOTIFY thread that relays status to SSE.
mod taskwatcher;
pub use taskwatcher::*;

mod task;
pub use task::*;

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
