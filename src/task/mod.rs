pub mod orchestrator;

mod gcloud_taskqueue;
pub use gcloud_taskqueue::*;

mod taskqueue;
pub use taskqueue::TaskQueue;

pub fn make_taskqueue(config: &crate::facility::Config) -> Box<dyn TaskQueue> {
    Box::new(PubSubHttpTaskQueue::from_config(config))
}
