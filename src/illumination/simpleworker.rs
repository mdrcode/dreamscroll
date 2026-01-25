use std::sync::Arc;

use crate::{
    api::{self, ApiError},
    common,
    database::DbHandle,
};

use super::*;

pub struct SimpleWorker {
    pub db: Arc<DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illuminator: Box<dyn Illuminator>,
}

impl Clone for SimpleWorker {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            queue: Arc::clone(&self.queue),
            illuminator: dyn_clone::clone(&self.illuminator),
        }
    }
}

impl SimpleWorker {
    pub fn new(db: Arc<DbHandle>, illuminator: Box<dyn Illuminator>) -> Self {
        Self {
            db,
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
            let capture_ids = api::fetch_captures_need_illumination(&self.db).await?;
            let n = capture_ids.len();
            let nq = self.queue.enqueue_iter(capture_ids);

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
        let db = &self.parent_arc.db;
        let queue = &self.parent_arc.queue;
        let illuminator = &self.parent_arc.illuminator;

        loop {
            if let Some(cap_id) = queue.pop_next() {
                tracing::info!("Starting illumination for capture ID {}...", cap_id);
                let capture = api::fetch_capture_by_id(&db, cap_id).await?;

                let r_illumination = illuminator.illuminate(capture.clone()).await;
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

                let r_insert = api::insert_illumination(db, cap_id, i).await;
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
