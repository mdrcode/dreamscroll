use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;

use super::{Illuminator, IlluminatorWorker};
use crate::{
    common::{self, AppError},
    controller::{CaptureInfo, IlluminationInfo},
    database::DbHandle,
};

#[derive(Clone)]
pub struct SimpleWorker<I: Illuminator + 'static> {
    pub db: Arc<DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illuminator: I,
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
    async fn run(&self) -> anyhow::Result<(), AppError> {
        let arc_self = Arc::new(self.clone());
        (0..2).for_each(|_| {
            let t = SimpleWorkerThread::new(Arc::clone(&arc_self));
            tokio::spawn(t.run());
        });

        loop {
            let capture_ids = CaptureInfo::fetch_ids_need_illumination(&self.db).await?;
            let n = capture_ids.len();

            let nq = self.queue.enqueue_iter(capture_ids);

            tracing::info!("Retrieved {} needing illumination, enqueued {}.", n, nq);

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

struct SimpleWorkerThread<I: Illuminator + 'static> {
    parent_arc: Arc<SimpleWorker<I>>,
}

impl<I: Illuminator + 'static> SimpleWorkerThread<I> {
    pub fn new(parent_arc: Arc<SimpleWorker<I>>) -> Self {
        Self { parent_arc }
    }
}

impl<I: Illuminator + 'static> SimpleWorkerThread<I> {
    // note this consumes self
    async fn run(self) -> anyhow::Result<(), AppError> {
        let parent = &self.parent_arc;

        loop {
            if let Some(capture_id) = parent.queue.pop_next() {
                let capture = CaptureInfo::fetch_by_id(&parent.db, capture_id).await?;

                let illumination = parent.illuminator.illuminate(capture).await;
                IlluminationInfo::insert(&parent.db, capture_id, "simpleTODOTDO", &illumination)
                    .await?;

                parent.queue.complete(capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
