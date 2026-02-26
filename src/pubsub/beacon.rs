use std::sync::Arc;

use super::*;

#[derive(Clone, Default)]
pub struct Beacon {
    illumination_queue: Option<Arc<dyn TopicQueue>>,
}

impl Beacon {
    pub fn builder() -> BeaconBuilder {
        BeaconBuilder::default()
    }

    pub async fn signal_new_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        if let Some(queue) = &self.illumination_queue {
            queue.enqueue(capture_id).await?;
            tracing::info!(capture_id, "Enqueued for illumination");
        } else {
            tracing::info!(
                capture_id,
                "New capture created but no illumination queue configured, skipping."
            );
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct BeaconBuilder {
    illumination_queue: Option<Arc<dyn TopicQueue>>,
}

impl BeaconBuilder {
    pub fn new_capture_topic(mut self, illumination_queue: impl TopicQueue + 'static) -> Self {
        self.illumination_queue = Some(Arc::new(illumination_queue));
        self
    }

    pub fn build(self) -> Beacon {
        Beacon {
            illumination_queue: self.illumination_queue,
        }
    }
}
