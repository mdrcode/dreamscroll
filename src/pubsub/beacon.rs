use std::sync::Arc;

use crate::webhook::r_wh_illuminate::IlluminationPayload;

use super::*;

#[derive(Clone, Default)]
pub struct Beacon {
    illumination_queue: Option<Arc<PubSubTopicQueue<IlluminationPayload>>>,
}

impl Beacon {
    pub fn builder() -> BeaconBuilder {
        BeaconBuilder::default()
    }

    pub async fn signal_new_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        if let Some(queue) = self.illumination_queue.as_ref() {
            queue.enqueue(IlluminationPayload { capture_id }).await.inspect_err(
                |err| tracing::error!(queue = ?queue, capture_id, error = ?err, "Failed to enqueue capture for illumination"),
            )?;
        } else {
            tracing::warn!("New capture created but no topic configured, skipping enqueue.");
        }

        Ok(())
    }
}

#[derive(Default)]
pub struct BeaconBuilder {
    illumination_queue: Option<Arc<PubSubTopicQueue<IlluminationPayload>>>,
}

impl BeaconBuilder {
    pub fn new_capture_topic(
        mut self,
        illumination_queue: PubSubTopicQueue<IlluminationPayload>,
    ) -> Self {
        self.illumination_queue = Some(Arc::new(illumination_queue));
        self
    }

    pub fn build(self) -> Beacon {
        Beacon {
            illumination_queue: self.illumination_queue,
        }
    }
}
