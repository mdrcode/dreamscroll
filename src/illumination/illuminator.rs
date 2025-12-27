use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::EntityTrait;

use crate::model::capture;
use crate::{common, database};

#[async_trait]
pub trait Illuminator: Send + Sync {
    async fn run(&self) -> anyhow::Result<()>;
}

pub fn make(db: Arc<database::DbHandle>) -> Box<dyn Illuminator> {
    Box::new(SimpleIlluminator {
        db,
        queue: common::OneShotQueue::new(),
    })
}

pub struct SimpleIlluminator {
    db: Arc<database::DbHandle>,
    queue: common::OneShotQueue<i32>,
}

#[async_trait]
impl Illuminator for SimpleIlluminator {
    async fn run(&self) -> anyhow::Result<()> {
        println!("Illuminator starting...");

        loop {
            // Fetch all captures from the database
            let captures = capture::Entity::find()
                .all(&self.db.conn)
                .await
                .expect("Failed to fetch captures");

            println!("Found {} captures in the database.", captures.len());

            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    }
}
