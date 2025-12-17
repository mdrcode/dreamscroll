use crate::db;
use crate::entity::capture;
use crate::facility::Facility;
use sea_orm::EntityTrait;
use std::sync::Arc;

pub async fn main_loop(db: Arc<db::DbHandle>, _: Box<dyn Facility>) {
    println!("Worker process starting...");

    // Example: Process images periodically
    loop {
        // Fetch all captures from the database
        let captures = capture::Entity::find().all(&db.conn).await;

        match captures {
            Ok(captures) => println!("Found {} captures in db", captures.len()),
            Err(e) => eprintln!("Failed to fetch captures: {}", e),
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
