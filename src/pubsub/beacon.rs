use std::sync::Arc;

use crate::webhook::r_wh_illuminate::IlluminationPayload;

use super::TopicQueue;

#[derive(Clone, Default)]
pub struct Beacon {
    illumination_queue: Option<Arc<dyn TopicQueue<Payload = IlluminationPayload>>>,
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
    illumination_queue: Option<Arc<dyn TopicQueue<Payload = IlluminationPayload>>>,
}

impl BeaconBuilder {
    pub fn new_capture_topic(
        mut self,
        illumination_queue: impl TopicQueue<Payload = IlluminationPayload> + 'static,
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::webhook::r_wh_illuminate::IlluminationPayload;

    use super::*;

    #[derive(Debug, Clone)]
    struct RecordingQueue {
        captures: Arc<Mutex<Vec<i32>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl TopicQueue for RecordingQueue {
        type Payload = IlluminationPayload;

        async fn enqueue(&self, payload: Self::Payload) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("enqueue failed")
            }

            let mut captures = self
                .captures
                .lock()
                .expect("RecordingQueue captures mutex should not be poisoned");
            captures.push(payload.capture_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn signal_new_capture_enqueues_payload() {
        let captures = Arc::new(Mutex::new(Vec::new()));
        let queue = RecordingQueue {
            captures: Arc::clone(&captures),
            fail: false,
        };

        let beacon = Beacon::builder().new_capture_topic(queue).build();

        beacon
            .signal_new_capture(42)
            .await
            .expect("signal_new_capture should succeed");

        let recorded = captures
            .lock()
            .expect("captures mutex should not be poisoned")
            .clone();
        assert_eq!(recorded, vec![42]);
    }

    #[tokio::test]
    async fn signal_new_capture_without_queue_is_noop() {
        let beacon = Beacon::default();

        beacon
            .signal_new_capture(7)
            .await
            .expect("signal_new_capture should be a no-op when queue is absent");
    }

    #[tokio::test]
    async fn signal_new_capture_propagates_enqueue_error() {
        let queue = RecordingQueue {
            captures: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        };
        let beacon = Beacon::builder().new_capture_topic(queue).build();

        let result = beacon.signal_new_capture(9).await;
        assert!(result.is_err());
    }
}
