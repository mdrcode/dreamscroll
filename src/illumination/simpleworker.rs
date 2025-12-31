use std::sync::Arc;

use async_trait::async_trait;

use super::{Illuminator, IlluminatorWorker};
use crate::{
    common::{self, AppError},
    controller::CaptureInfo,
    database::DbHandle,
    model::illumination,
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
        let self_arc = Arc::new(self.clone());
        (0..2).for_each(|_| {
            let t = SimpleWorkerThread {
                parent_arc: self_arc.clone(),
            };
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
    pub parent_arc: Arc<SimpleWorker<I>>,
}

impl<I: Illuminator + 'static> SimpleWorkerThread<I> {
    // note this consumes self
    async fn run(self) -> anyhow::Result<(), AppError> {
        let db = &self.parent_arc.db;
        let queue = &self.parent_arc.queue;
        let illuminator = &self.parent_arc.illuminator;

        loop {
            if let Some(capture_id) = queue.pop_next() {
                let capture = CaptureInfo::fetch_by_id(&db, capture_id).await?;

                let i = illuminator.illuminate(capture).await?;

                illumination::ActiveModel::builder()
                    .set_capture_id(capture_id)
                    .set_provider("simpleTODOTODO".to_string())
                    .set_content(i)
                    .save(&db.conn)
                    .await?;

                queue.complete(capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
