use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use dreamscroll::{api, auth, database, facility, illumination, rest, storage, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::from_filename("ds_config.env").ok();
    let _ = dotenvy::from_filename("ds_secrets.env"); // gitignored for api keys

    let config = facility::make_config();
    facility::init_tracing(&config);

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    facility::check_first_users(&db).await?;

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let api_client = api::ApiClient::new(db.clone(), stg.clone(), url_maker);

    let jwt_secret = config.jwt_secret.unwrap_or_else(|| {
        tracing::warn!("JWT secret not set, using default for localdev. NOT FOR PROD!");
        "dreamscroll-local-jwt-secret-not-for-prod".to_string()
    });
    let jwt = auth::JwtConfig::from_secret(jwt_secret.as_bytes());

    let cancel_token = CancellationToken::new();

    let auth_backend = auth::WebAuthBackend::new(db.clone());

    let thread_webui = {
        // Web UI routes (Session-auth protected) + static JS/CSS serving
        let mut router = webui::v1::make_ui_router(api_client.clone(), session_store, auth_backend);

        // REST API routes (JWT-protected)
        let api_router = rest::make_api_router(api_client.clone(), jwt.clone());
        router = router.nest("/api", api_router);

        // Web serving for media assets stored with the local storage provider
        router = router.nest_service(
            &config.storage_local_url_prefix,
            ServeDir::new(&config.storage_local_file_path),
        );

        let cancel = cancel_token.clone();
        let host_port = format!("0.0.0.0:{}", config.web_port);
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
        config.web_port
    );

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

    let _ = webbrowser::open(&format!("http://localhost:{}", config.web_port));

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_webui, thread_illuminator);

    Ok(())
}
