use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{EntityTrait, QuerySelect};

use crate::model::capture;
use crate::{common, database};

type Illumination = fn(capture_id: i32) -> String;

pub fn make(db: Arc<database::DbHandle>, ill: Illumination) -> Box<dyn Illuminator> {
    Box::new(SimpleIlluminator {
        db,
        queue: Arc::new(common::OneShotQueue::new()),
        illumination: ill,
    })
}

#[async_trait]
pub trait Illuminator: Send + Sync {
    async fn run(&self) -> anyhow::Result<()>;
}

pub struct SimpleIlluminator {
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
    illumination: Illumination,
}

#[async_trait]
impl Illuminator for SimpleIlluminator {
    async fn run(&self) -> anyhow::Result<()> {
        (0..2).for_each(|_| {
            let t = IlluminationThread {
                db: self.db.clone(),
                queue: self.queue.clone(),
                illumination: self.illumination,
            };
            tokio::spawn(t.run());
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

struct IlluminationThread {
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
    illumination: Illumination,
}

impl IlluminationThread {
    // note this consumes self
    async fn run(self) {
        loop {
            if let Some(capture_id) = self.queue.pop_next() {
                (self.illumination)(capture_id);
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                self.queue.complete(capture_id);
                println!("Worker thread illuminated capture ID {}", capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
