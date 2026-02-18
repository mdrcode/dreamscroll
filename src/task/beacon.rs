use std::sync::Arc;

use super::*;

#[derive(Clone)]
pub struct Beacon {
    illumination_queue: Option<Arc<dyn TaskQueue>>,
}

impl Beacon {
    pub fn builder() -> BeaconBuilder {
        BeaconBuilder::default()
    }

    pub async fn signal_new_capture(&self, capture_id: i32) -> anyhow::Result<()> {
        if let Some(queue) = &self.illumination_queue {
            queue.enqueue(capture_id).await?;
        } else {
            tracing::warn!(
                capture_id,
                "New capture created but no illumination queue configured, skipping enqueue."
            );
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct BeaconBuilder {
    illumination_queue: Option<Arc<dyn TaskQueue>>,
}

impl BeaconBuilder {
    pub fn illumination_queue(mut self, illumination_queue: Box<dyn TaskQueue>) -> Self {
        self.illumination_queue = Some(Arc::from(illumination_queue));
        self
    }

    pub fn build(self) -> Beacon {
        Beacon {
            illumination_queue: self.illumination_queue,
        }
    }
}
