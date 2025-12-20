use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamspot::{config, db, storage, webui, worker};

#[tokio::main]
async fn main() {
    let (db_config, storage_config) = config::make_local_dev();
    let webui_host_port = "127.0.0.1:8000".to_string();

    let db = db::connect(db_config).await.unwrap();
    db::run_migrations(&db).await.unwrap();
    let db = Arc::new(db);

    let storage = storage::make(storage_config);
    let storage: Arc<dyn storage::StorageProvider> = Arc::from(storage);

    let cancel_token = CancellationToken::new();

    let webui_router = webui::build_axum_router(db.clone(), storage.clone());
    let webui_cancel = cancel_token.clone();
    let host_port_clone = webui_host_port.clone();
    let h_webui = tokio::spawn(async move {
        let listener = TcpListener::bind(host_port_clone).await.unwrap();
        axum::serve(listener, webui_router)
            .with_graceful_shutdown(async move {
                webui_cancel.cancelled().await;
            })
            .await
            .unwrap();
    });
    println!("Web UI serving at http://{}", webui_host_port);

    let worker_cancel = cancel_token.clone();
    let worker_db = db.clone();
    let h_worker = tokio::spawn(async move {
        tokio::select! {
            _ = worker_cancel.cancelled() => {}
            _ = worker::main_loop(worker_db) => {}
        }
    });

    let _ = webbrowser::open(&format!("http://{}", webui_host_port));

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(h_webui, h_worker);
}
