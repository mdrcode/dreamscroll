// Beacon is the unifying abstraction for task queues
mod beacon;
pub use beacon::*;

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
