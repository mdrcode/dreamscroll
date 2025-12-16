use std::sync::Arc;

use dreamspot::{db, worker};

#[tokio::main]
async fn main() {
    let db_config = db::DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };

    let db = db::connect(db_config).await.unwrap();
    let db = Arc::new(db);

    db::run_migrations(&db).await.unwrap();

    let rocket = dreamspot::webui::build_rocket(db);
    let h_rocket = tokio::spawn(async move {
        println!(
            "Spawning Rocket server on thread: {}...",
            std::thread::current().name().unwrap_or("unknown")
        );
        let _ = rocket.launch().await;
    });

    let h_worker = tokio::spawn(async move {
        println!(
            "Spawning worker on thread: {}...",
            std::thread::current().name().unwrap_or("unknown")
        );
        worker::main_loop().await;
    });

    // Wait for CTRL-C
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nReceived CTRL-C, shutting down...");
        }
        _ = h_rocket => {
            println!("Rocket server stopped");
        }
        _ = h_worker => {
            println!("Worker stopped");
        }
    }
}
