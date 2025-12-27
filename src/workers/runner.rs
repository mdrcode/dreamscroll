use std::sync::Arc;

use sea_orm::EntityTrait;

use crate::database;
use crate::model::capture;



pub struct Runner {
    db: Arc<database::DbHandle>,
}

impl Runner {
    pub fn new(db: Arc<database::DbHandle>) -> Self {
        Self { db }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        println!("Worker loop starting...");

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
