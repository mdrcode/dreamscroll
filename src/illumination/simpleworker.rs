use std::sync::Arc;

use async_trait::async_trait;

use super::{Illuminator, IlluminatorWorker};
use crate::{api, common, database::DbHandle};

pub struct SimpleWorker<I: Illuminator + 'static> {
    pub db: Arc<DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illuminator: I,
}

impl<I> Clone for SimpleWorker<I>
where
    I: Illuminator + 'static,
{
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            queue: Arc::clone(&self.queue),
            illuminator: dyn_clone::clone(&self.illuminator),
        }
    }
}

impl<I> SimpleWorker<I>
where
    I: Illuminator + 'static,
{
    pub fn new(db: Arc<DbHandle>, illuminator: I) -> Self {
        Self {
            db,
            queue: Arc::new(common::OneShotQueue::new()),
            illuminator,
        }
    }
}

#[async_trait]
impl<I: Illuminator + 'static> IlluminatorWorker for SimpleWorker<I> {
    async fn run(&self) -> anyhow::Result<(), api::AppError> {
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

struct SimpleWorkerThread<I: Illuminator + 'static> {
    pub parent_arc: Arc<SimpleWorker<I>>,
}

impl<I: Illuminator + 'static> SimpleWorkerThread<I> {
    // note this consumes self
    async fn run(self) -> anyhow::Result<(), api::AppError> {
        let db = &self.parent_arc.db;
        let queue = &self.parent_arc.queue;
        let illuminator = &self.parent_arc.illuminator;

        loop {
            if let Some(cap_id) = queue.pop_next() {
                let capture = api::fetch_capture_by_id(&db, cap_id).await?;

                let i = illuminator.illuminate(capture).await?;

                api::insert_illumination(db, cap_id, illuminator, i).await?;

                queue.complete(cap_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
