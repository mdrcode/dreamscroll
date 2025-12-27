use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{EntityTrait, QuerySelect};

use super::{Illumination, IlluminationWorker};
use crate::{common, database, model};

#[derive(Clone)]
pub struct SimpleWorker<I: Illumination + 'static> {
    pub db: Arc<database::DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illumination: I,
}

#[async_trait]
impl<I: Illumination + 'static> IlluminationWorker for SimpleWorker<I> {
    async fn run(&self) -> anyhow::Result<()> {
        let arc_self = Arc::new(self.clone());
        (0..2).for_each(|_| {
            let t = SimpleWorkerThread::new(Arc::clone(&arc_self));
            tokio::spawn(t.run());
        });

        loop {
            let captures = model::capture::Entity::find()
                .select_only()
                .column(model::capture::Column::Id)
                .into_tuple::<i32>()
                .all(&self.db.conn)
                .await
                .expect("Failed to fetch capture IDs");

            let n = self.queue.enqueue_iter(&captures);

            tracing::info!("Found {} total caps in db, enqueued {}.", captures.len(), n);

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

struct SimpleWorkerThread<I: Illumination + 'static> {
    parent_arc: Arc<SimpleWorker<I>>,
}

impl<I: Illumination + 'static> SimpleWorkerThread<I> {
    pub fn new(parent_arc: Arc<SimpleWorker<I>>) -> Self {
        Self { parent_arc }
    }
}

impl<I: Illumination + 'static> SimpleWorkerThread<I> {
    // note this consumes self
    async fn run(self) {
        let parent = &self.parent_arc;

        loop {
            if let Some(capture_id) = parent.queue.pop_next() {
                parent.illumination.illuminate(capture_id).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                parent.queue.complete(capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
