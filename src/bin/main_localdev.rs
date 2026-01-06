use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamspot::{database, facility, illumination, storage, webui_v1};

#[tokio::main]
async fn main() {
    let config = facility::make_config(facility::Env::LocalDev);
    facility::init_logging(&config);

    let db = database::connect(config.db_config).await.unwrap();
    let db = Arc::new(db);

    let storage = storage::make(config.storage_config);
    let storage: Arc<dyn storage::StorageProvider> = Arc::from(storage);

    let cancel_token = CancellationToken::new();

    let webui_host_port = config
        .webui_host_port
        .expect("webui_host_port must be set in config for local hosting");

    let thread_webui = {
        let router = webui_v1::make_axum_router(db.clone(), storage.clone());
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

    let thread_illuminator = {
        let gemini = illumination::GeminiIlluminator::default();
        let worker = illumination::make_worker(db.clone(), gemini);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = worker.run() => {}
                _ = cancel.cancelled() => {}
            }
        })
    };

    let _ = webbrowser::open(&format!("http://{}", webui_host_port));

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_webui, thread_illuminator);
}
