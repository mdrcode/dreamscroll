pub mod orchestrator;

mod gcloud_taskqueue;
pub use gcloud_taskqueue::*;

mod taskqueue;
pub use taskqueue::TaskPublisher;

pub fn make_taskqueue(config: &crate::facility::Config) -> Box<dyn TaskPublisher> {
    Box::new(PubSubHttpTaskPublisher::from_config(config))
}
