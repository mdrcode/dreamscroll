use std::sync::Arc;

use crate::{api, common, illumination, webhook::r_wh_illuminate};

use super::*;

pub struct SimpleWorker {
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
    queue: Arc<common::OneShotQueue<i32>>,
}

impl Clone for SimpleWorker {
    fn clone(&self) -> Self {
        Self {
            service_api: self.service_api.clone(),
            illuminator: self.illuminator.clone(),
            queue: Arc::clone(&self.queue),
        }
    }
}

impl SimpleWorker {
    pub fn new(service_api: api::ServiceApiClient, illuminator: Box<dyn illumination::Illuminator>) -> Self {
        Self {
            service_api,
            illuminator,
            queue: Arc::new(common::OneShotQueue::new()),
        }
    }
}

#[async_trait::async_trait]
impl IlluminatorWorker for SimpleWorker {
    async fn run(&self) -> anyhow::Result<(), api::ApiError> {
        let self_arc = Arc::new(self.clone());
        (0..2).for_each(|_| {
            let t = SimpleWorkerThread {
                parent_arc: Arc::clone(&self_arc),
            };
            tokio::spawn(t.run());
        });

        loop {
            let ids = self.service_api.get_captures_need_illum().await?;
            let n = ids.len();
            let nq = self.queue.enqueue_iter(ids);

            if nq > 0 {
                tracing::info!("Retrieved {} needing illumination, enqueued {}.", n, nq);
            } else {
                tracing::debug!(
                    "Retrieved {} needing illumination, none enqueued (all already queued).",
                    n
                );
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}

struct SimpleWorkerThread {
    pub parent_arc: Arc<SimpleWorker>,
}

impl SimpleWorkerThread {
    // note this consumes self
    async fn run(self) -> anyhow::Result<(), api::ApiError> {
        let queue = &self.parent_arc.queue;

        loop {
            if let Some(cap_id) = queue.pop_next() {
                match r_wh_illuminate::execute(
                    &self.parent_arc.service_api,
                    &self.parent_arc.illuminator,
                    cap_id,
                )
                .await
                {
                    Ok(()) => {
                        queue.complete(cap_id);
                    }
                    Err(err) => {
                        tracing::error!(
                            capture_id = cap_id,
                            error = ?err,
                            "Failed processing capture in local worker"
                        );
                    }
                }
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
