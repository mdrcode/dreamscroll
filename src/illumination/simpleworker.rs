use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{EntityTrait, QuerySelect};

use super::{Illumination, IlluminationWorker};
use crate::{common, database, model};

pub struct SimpleWorker<I: Illumination + 'static> {
    pub db: Arc<database::DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illumination: I,
}

#[async_trait]
impl<I: Illumination + 'static> IlluminationWorker for SimpleWorker<I> {
    async fn run(&self) -> anyhow::Result<()> {
        (0..2).for_each(|_| {
            let t = SimpleWorkerThread {
                db: self.db.clone(),
                queue: self.queue.clone(),
                illumination: self.illumination.clone(),
            };
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
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
    illumination: I,
}

impl<I: Illumination + 'static> SimpleWorkerThread<I> {
    // note this consumes self
    async fn run(self) {
        loop {
            if let Some(capture_id) = self.queue.pop_next() {
                self.illumination.illuminate(capture_id).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                self.queue.complete(capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
