use dreamspot::{db, worker};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let db_config = db::DbConfig::SqliteFile {
        path: "localdev/dreamspot.db".to_string(),
    };
    let db = db::connect(db_config).await.unwrap();
    let db = Arc::new(db);
    db::run_migrations(&db).await.unwrap();

    let cancel_token = CancellationToken::new();

    let web_rocket = dreamspot::webui::build_rocket(db);
    let web_token = cancel_token.clone();
    let h_web = tokio::spawn(async move {
        tokio::select! {
            _ = web_token.cancelled() => {}
            _ = web_rocket.launch() => {}
        }
    });

    let worker_token = cancel_token.clone();
    let h_worker = tokio::spawn(async move {
        tokio::select! {
            _ = worker_token.cancelled() => {}
            _ = worker::main_loop() => {}
        }
    });

    tokio::signal::ctrl_c().await.unwrap();
    cancel_token.cancel();
    let _ = tokio::join!(h_web, h_worker);
}
