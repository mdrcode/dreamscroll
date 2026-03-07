
mod taskqueue;
pub use taskqueue::*;

// A handle which converts logical signals (e.g. "new capture 323") into
// concrete tasks (e.g. "enqueue capture 323 for illumination")
mod beacon;
pub use beacon::*;

// Gcloud Pub/Sub implementation of TaskQueue
pub mod gcloud;
pub use gcloud::*;
