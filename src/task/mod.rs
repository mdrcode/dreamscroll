// A handle which converts logical signals (e.g. "new capture 323") into
// concrete tasks (e.g. "enqueue capture 323 for illumination")
mod beacon;
pub use beacon::*;

mod taskqueue;
pub use taskqueue::*;

mod taskqueue_firestore;
pub use taskqueue_firestore::*;

pub mod taskqueue_pubsub;
pub use taskqueue_pubsub::*;

pub mod util;
