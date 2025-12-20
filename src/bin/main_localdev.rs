use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamspot::db;
use dreamspot::facility::{Environment, make_facility};
use dreamspot::storage::{StorageProvider, make_storage};
use dreamspot::webui;
use dreamspot::worker;

#[tokio::main]
async fn main() {
    let facility = make_facility(Environment::LocalDev);

    let db = db::connect(facility.db_config()).await.unwrap();
    db::run_migrations(&db).await.unwrap();
    let db = Arc::new(db);

    let storage = make_storage(facility.storage_config());
    let storage: Arc<dyn StorageProvider> = Arc::from(storage);

    let cancel_token = CancellationToken::new();

    let webui_router = webui::build_axum_router(db.clone(), storage.clone());
    let webui_cancel = cancel_token.clone();
    let webui_fac = facility.clone();
    let h_webui = tokio::spawn(async move {
        let listener = TcpListener::bind(&webui_fac.ui_host_port()).await.unwrap();
        axum::serve(listener, webui_router)
            .with_graceful_shutdown(async move {
                webui_cancel.cancelled().await;
            })
            .await
            .unwrap();
    });
    println!("Web UI serving at http://{}", facility.ui_host_port());

    let worker_cancel = cancel_token.clone();
    let worker_db = db.clone();
    let worker_fac = facility.clone();
    let h_worker = tokio::spawn(async move {
        tokio::select! {
            _ = worker_cancel.cancelled() => {}
            _ = worker::main_loop(worker_db, worker_fac) => {}
        }
    });

    let _ = webbrowser::open(&format!("http://{}", facility.ui_host_port()));

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(h_webui, h_worker);
}
