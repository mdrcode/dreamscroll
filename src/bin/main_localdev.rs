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

    facility::check_first_users(&db).await?;

    let stg = storage::make_provider(config.storage_config).await;
    let stg: Arc<dyn storage::StorageProvider> = Arc::from(stg);

    let jwt = Arc::new(config.jwt_config);

    let cancel_token = CancellationToken::new();

    let web_host_port = config
        .web_host_port
        .expect("webui_host_port must be configured");

    let thread_webui = {
        // Web UI routes (Session-auth protected) + static JS/CSS serving
        let mut router = webui::v1::make_ui_router(db.clone(), stg.clone());

        // REST API routes (JWT-protected)
        let api_router = rest::make_api_router(db.clone(), jwt.clone());
        router = router.nest("/api", api_router);

        // Check if the storage provider requires local web serving
        if let Some(serving) = stg.local_web_serving() {
            router = router.nest_service(&serving.web_path, ServeDir::new(serving.local_path));
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

    let illuminator_context = auth::Context::from_service_credentials(
        &jwt,
        // For local dev, there are no true secrets, so just create token on the fly
        jwt.create_service_token("illuminator_worker")?,
    )?;
    let thread_illuminator = {
        let gemini = illumination::make_illuminator("geministructured");
        let worker = illumination::make_worker(db.clone(), illuminator_context, gemini);
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
