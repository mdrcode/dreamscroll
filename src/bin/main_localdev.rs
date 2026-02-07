use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use dreamscroll::{api, auth, database, facility, illumination, rest, storage, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename("secrets.env"); // gitignored

    let config = facility::make_config(facility::Env::LocalDev);
    facility::init_tracing(&config);

    let pool = database::create_sqlite_pool(&config.sqlite_url).await?;
    let db_connection = database::connect_sqlite_db(pool.clone()).await?;
    let session_store = database::connect_sqlite_session_store(pool.clone()).await?;

    let db = database::DbHandle::new(db_connection);

    facility::check_first_users(&db).await?;

    let stg = storage::make_provider(config.storage.clone()).await;
    let url_maker = storage::UrlMaker::new(config.storage_url_maker.clone());
    let api_client = api::ApiClient::new(db.clone(), stg.clone(), url_maker);

    let jwt = config.jwt;

    let cancel_token = CancellationToken::new();

    let auth_backend = auth::WebAuthBackend::new(db.clone());

    let thread_webui = {
        // Web UI routes (Session-auth protected) + static JS/CSS serving
        let mut router = webui::v1::make_ui_router(api_client.clone(), session_store, auth_backend);

        // REST API routes (JWT-protected)
        let api_router = rest::make_api_router(api_client.clone(), jwt.clone());
        router = router.nest("/api", api_router);

        // Check if the storage provider requires local web serving
        if let Some(serving) = stg.local_web_serving() {
            router = router.nest_service(&serving.web_path, ServeDir::new(serving.file_path));
        }

        let cancel = cancel_token.clone();
        let host_port = format!("0.0.0.0:{}", config.port);
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
    println!("Web UI serving locally at http://localhost:{}", config.port);

    let illuminator_context = auth::Context::from_service_credentials(
        &jwt,
        // For local dev, there are no true secrets, so just create token on the fly
        jwt.create_service_token("illuminator_worker")?,
    )?;
    let thread_illuminator = {
        let gemini = illumination::make_illuminator("geministructured", api_client.clone());
        let worker = illumination::make_worker(api_client.clone(), illuminator_context, gemini);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = worker.run() => {}
                _ = cancel.cancelled() => {}
            }
        })
    };

    let _ = webbrowser::open(&format!("http://localhost:{}", config.port));

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_webui, thread_illuminator);

    Ok(())
}
