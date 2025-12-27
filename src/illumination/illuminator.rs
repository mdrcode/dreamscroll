use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::EntityTrait;

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
            tokio::spawn(illuminate_worker(self.db.clone(), self.queue.clone()));
        });

        loop {
            // Fetch all captures from the database
            let captures = capture::Entity::find()
                .all(&self.db.conn)
                .await
                .expect("Failed to fetch captures")
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<i32>>();

            self.queue.enqueue_iter(&captures);

            println!("Found {} captures in the database.", captures.len());

            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    }
}

async fn illuminate_worker(_db: Arc<database::DbHandle>, queue: Arc<common::OneShotQueue<i32>>) {
    loop {
        if let Some(capture_id) = queue.pop_next() {
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            queue.complete(capture_id);
            println!("Worker thread illuminated capture ID {}", capture_id);
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }
}
