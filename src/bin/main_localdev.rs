use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use dreamscroll::{database, facility, illumination, storage, webui};

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
        .expect("webui_host_port must be configured");

    let thread_webui = {
        let mut router = webui::v1::make_axum_router(db.clone(), storage.clone());

        // For local dev, we serve static files directly
        router = router.nest_service("/static", ServeDir::new("web_static/"));

        // ... and we serve media directly from local file storage
        let local_serving_path_opt = storage.local_serving_path();
        if let Some(ref path) = local_serving_path_opt {
            router = router.nest_service("/media", ServeDir::new(path));
        }

        let cancel = cancel_token.clone();
        let host_port = format!("{}:{}", webui_host_port.0, webui_host_port.1);
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
    println!(
        "Web UI serving locally at http://localhost:{}",
        webui_host_port.1
    );

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

    let _ = webbrowser::open(&format!("http://localhost:{}", webui_host_port.1));

    tokio::signal::ctrl_c().await.unwrap();
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_webui, thread_illuminator);
}
