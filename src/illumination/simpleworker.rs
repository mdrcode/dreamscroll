use std::sync::Arc;

use crate::{api, auth, common};

use super::*;

pub struct SimpleWorker {
    api: Arc<api::ApiClient>,
    context: auth::Context,
    queue: Arc<common::OneShotQueue<i32>>,
    illuminator: Box<dyn Illuminator>,
}

impl Clone for SimpleWorker {
    fn clone(&self) -> Self {
        Self {
            api: Arc::clone(&self.api),
            context: self.context.clone(),
            queue: Arc::clone(&self.queue),
            illuminator: dyn_clone::clone(&self.illuminator),
        }
    }
}

impl SimpleWorker {
    pub fn new(
        api_client: Arc<api::ApiClient>,
        context: auth::Context,
        illuminator: Box<dyn Illuminator>,
    ) -> Self {
        Self {
            api: api_client,
            context,
            queue: Arc::new(common::OneShotQueue::new()),
            illuminator,
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
            let ids = self.api.fetch_capture_for_illum(&self.context).await?;
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

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

struct SimpleWorkerThread {
    pub parent_arc: Arc<SimpleWorker>,
}

impl SimpleWorkerThread {
    // note this consumes self
    async fn run(self) -> anyhow::Result<(), api::ApiError> {
        let api = &self.parent_arc.api;
        let context = &self.parent_arc.context;
        let queue = &self.parent_arc.queue;
        let illuminator = &self.parent_arc.illuminator;

        loop {
            if let Some(cap_id) = queue.pop_next() {
                tracing::info!("Starting illumination for capture ID {}...", cap_id);
                let fetch = api.fetch_captures(&context, Some(vec![cap_id])).await?;

                let Some(capture) = fetch.into_iter().next() else {
                    tracing::error!("Capture ID {} not found during illumination.", cap_id);
                    continue;
                };

                let r_illumination = illuminator.illuminate(&capture).await;
                if r_illumination.is_err() {
                    let err = r_illumination.as_ref().err().unwrap();
                    tracing::error!(
                        "Error during illumination for capture ID {}: {:?}",
                        cap_id,
                        err
                    );
                    continue;
                }
                let i = r_illumination?;

                let r_insert = api.insert_illumination(&context, &capture, i).await;
                if r_insert.is_err() {
                    let err = r_insert.as_ref().err().unwrap();
                    tracing::error!(
                        "Error inserting illumination for capture ID {}: {:?}",
                        cap_id,
                        err
                    );
                    continue;
                }

                queue.complete(cap_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
