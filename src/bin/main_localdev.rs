use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use dreamscroll::{auth, database, facility, illumination, rest, storage, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = facility::make_config(facility::Env::LocalDev);
    facility::init_tracing(&config);

    let db = database::connect(config.db_config).await?;
    let db = Arc::new(db);

    let storage = storage::make(config.storage_config);
    let storage: Arc<dyn storage::StorageProvider> = Arc::from(storage);

    // Initialize JWT configuration from environment variable.
    // For local development, a default secret is used if not set.
    let jwt_config = Arc::new(
        std::env::var("JWT_SECRET")
            .map(|secret| auth::JwtConfig::from_secret(secret.as_bytes()))
            .unwrap_or_else(|_| {
                tracing::warn!(
                    "JWT_SECRET not set, using default development secret. \
                     DO NOT use this in production!"
                );
                auth::JwtConfig::from_secret(b"dreamscroll-local-dev-secret-key-not-for-production")
            }),
    );

    let cancel_token = CancellationToken::new();

    let web_host_port = config
        .web_host_port
        .expect("webui_host_port must be configured");

    let thread_webui = {
        let mut router = webui::v1::make_ui_router(db.clone(), storage.clone());

        // REST API routes (JWT-protected)
        let api_router = rest::make_api_router(db.clone(), jwt_config.clone());
        router = router.nest("/api", api_router);

        // For local dev, we serve static JS/CSS files directly
        router = router.nest_service("/static", ServeDir::new("web/v1/static"));

        // ... and we serve media directly from local file storage
        let local_serving_path_opt = storage.local_serving_path();
        if let Some(ref path) = local_serving_path_opt {
            router = router.nest_service("/media", ServeDir::new(path));
        }

        let cancel = cancel_token.clone();
        let host_port = format!("{}:{}", web_host_port.0, web_host_port.1);
        tokio::spawn(async move {
            let listener = TcpListener::bind(host_port).await.unwrap();
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    cancel.cancelled().await;
                })
                .await
                .expect("Failed to serve Web.");
        })
    };
    println!(
        "Web UI serving locally at http://localhost:{}",
        web_host_port.1
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

    let _ = webbrowser::open(&format!("http://localhost:{}", web_host_port.1));

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_webui, thread_illuminator);

    Ok(())
}
