use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::{EntityTrait, QuerySelect};

use super::{Illumination, Illuminator};
use crate::model::capture;
use crate::{common, database};

pub struct SimpleIlluminator<I: Illumination + 'static> {
    pub db: Arc<database::DbHandle>,
    pub queue: Arc<common::OneShotQueue<i32>>,
    pub illumination: I,
}

#[async_trait]
impl<I: Illumination + 'static> Illuminator for SimpleIlluminator<I> {
    async fn run(&self) -> anyhow::Result<()> {
        (0..2).for_each(|_| {
            let t = SimpleIlluminatorThread {
                db: self.db.clone(),
                queue: self.queue.clone(),
                illumination: self.illumination.clone(),
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

            let enqueued = self.queue.enqueue_iter(&captures);

            tracing::info!(
                "Found {} total captures in db, enqueued {}.",
                captures.len(),
                enqueued
            );

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
}

struct SimpleIlluminatorThread<I: Illumination + 'static> {
    db: Arc<database::DbHandle>,
    queue: Arc<common::OneShotQueue<i32>>,
    illumination: I,
}

impl<I: Illumination + 'static> SimpleIlluminatorThread<I> {
    // note this consumes self
    async fn run(self) {
        loop {
            if let Some(capture_id) = self.queue.pop_next() {
                self.illumination.illuminate(capture_id).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                self.queue.complete(capture_id);
                println!("Worker thread illuminated capture ID {}", capture_id);
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }
    }
}
