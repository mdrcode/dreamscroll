use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamspot::{db, facility::*, webui, worker};

#[tokio::main]
async fn main() {
    let facility = make_facility(Environment::LocalDev);

    let db = db::connect(facility.db_config()).await.unwrap();
    db::run_migrations(&db).await.unwrap();
    let db = Arc::new(db);

    let cancel_token = CancellationToken::new();

    let webui_router = webui::build_axum_router(db.clone(), facility.clone());
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
