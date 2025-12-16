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

    let web = dreamspot::webui::build_rocket_web(db);
    let web_cancel = cancel_token.clone();
    let h_web = tokio::spawn(async move {
        tokio::select! {
            _ = web_cancel.cancelled() => {}
            _ = web.launch() => {}
        }
    });

    let worker_cancel = cancel_token.clone();
    let h_worker = tokio::spawn(async move {
        tokio::select! {
            _ = worker_cancel.cancelled() => {}
            _ = worker::main_loop() => {}
        }
    });

    tokio::signal::ctrl_c().await.unwrap();
    cancel_token.cancel();
    let _ = tokio::join!(h_web, h_worker);
}
