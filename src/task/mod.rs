pub mod orchestrator;

mod beacon;
pub use beacon::*;

mod gcloud_taskqueue;
pub use gcloud_taskqueue::*;

mod taskqueue;
pub use taskqueue::*;

pub fn make_taskqueue(config: &crate::facility::Config) -> Box<dyn TaskQueue> {
    Box::new(PubSubHttpTaskQueue::from_config(config))
}
