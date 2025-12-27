use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{EntityTrait, QuerySelect};

use crate::model::capture;
use crate::{common, database};

pub fn make(db: Arc<database::DbHandle>) -> Box<dyn Illuminator> {
    Box::new(SimpleIlluminator {
        db,
        queue: Arc::new(common::OneShotQueue::new()),
    })
}

#[async_trait]
pub trait Illuminator: Send + Sync {
    async fn run(&self) -> anyhow::Result<()>;
}

pub struct SimpleIlluminator {
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
}

#[async_trait]
impl Illuminator for SimpleIlluminator {
    async fn run(&self) -> anyhow::Result<()> {
        (0..2).for_each(|_| {
            let worker = IlluminateWorker {
                db: self.db.clone(),
                queue: self.queue.clone(),
            };
            tokio::spawn(worker.run());
        });

        loop {
            let captures = capture::Entity::find()
                .select_only()
                .column(capture::Column::Id)
                .into_tuple::<i32>()
                .all(&self.db.conn)
                .await
                .expect("Failed to fetch capture IDs");

            self.queue.enqueue_iter(&captures);

            tracing::info!("Found {} captures in the database.", captures.len());

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

struct IlluminateWorker {
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
}

impl IlluminateWorker {
    // note this consumes self
    async fn run(self) {
        loop {
            if let Some(capture_id) = self.queue.pop_next() {
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                self.queue.complete(capture_id);
                println!("Worker thread illuminated capture ID {}", capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
