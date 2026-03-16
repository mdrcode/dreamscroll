// A handle which converts logical signals (e.g. "new capture 323") into
// concrete tasks (e.g. "enqueue capture 323 for illumination")
mod beacon;
pub use beacon::*;

mod taskqueue;
pub use taskqueue::*;

mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;

mod taskqueue_local;
pub use taskqueue_local::*;

pub mod taskqueue_pubsub;
pub use taskqueue_pubsub::*;

use serde::Deserialize;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskQueueBackend {
    Local,
    GCloudPubSub,
    GCloudTasks,
}
