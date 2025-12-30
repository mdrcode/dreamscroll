use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamspot::{config, database, illumination, storage, webui};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        //.with_max_level(tracing::Level::WARN)
        .init();

    let (db_config, storage_config) = config::make(config::Env::LocalDev);
    let webui_host_port = "127.0.0.1:8000".to_string();

    let db = database::connect(db_config).await.unwrap();
    let db = Arc::new(db);

    let storage = storage::make(storage_config);
    let storage: Arc<dyn storage::StorageProvider> = Arc::from(storage);

    let cancel_token = CancellationToken::new();

    let h_webui = {
        let router = webui::make_axum_router(db.clone(), storage.clone());
        let cancel = cancel_token.clone();
        let host_port = webui_host_port.clone();
        tokio::spawn(async move {
            let listener = TcpListener::bind(host_port).await.unwrap();
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    cancel.cancelled().await;
                })
                .await
                .expect("Failed to serve Web UI.");
        })
    };
    println!("Web UI serving at http://{}", webui_host_port);

    let h_illuminator = {
        let grok = illumination::GrokIlluminator {};
        let illuminator = illumination::make_worker(db.clone(), grok);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = illuminator.run() => {}
                _ = cancel.cancelled() => {}
            }
        })
    };

    let _ = webbrowser::open(&format!("http://{}", webui_host_port));

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(h_webui, h_illuminator);
}
