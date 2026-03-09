// A handle which converts logical signals (e.g. "new capture 323") into
// concrete tasks (e.g. "enqueue capture 323 for illumination")
mod beacon;
pub use beacon::*;

mod taskqueue;
pub use taskqueue::*;

mod taskqueue_cloudtask;
pub use taskqueue_cloudtask::*;

pub mod taskqueue_pubsub;
pub use taskqueue_pubsub::*;

pub mod util;
