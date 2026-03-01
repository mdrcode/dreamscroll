use anyhow::Context;
use rustls::crypto;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

use dreamscroll::{
    api, auth, database, facility, illumination, pubsub, rest, storage, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=(any value).
    // But when running via `cargo run` we load local files for convenience.
    if !std::env::var("NO_LOCAL_CONFIG_FILES").is_ok() {
        facility::load_local_config_files();
    }

    facility::init_tracing().await?;
    let config = facility::make_config()?;

    tracing::info!(
        "Starting dreamscroll_web with services: {:?}",
        config.services
    );

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    facility::check_users(&db).await?;

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::from_config(&config);
    let new_capture_topic = pubsub::PubSubTopicQueue::connect(
        config.pubsub_emulator.as_deref(),
        config.gcloud_project_id.as_str(),
        config.pubsub_topic_id_new_capture.as_str(),
    )
    .await
    .context("Failed to initialize Pub/Sub topic queue")?;
    let beacon = pubsub::Beacon::builder()
        .new_capture_topic(new_capture_topic)
        .build();
    let user_api = api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), beacon);
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    tracing::info!("Initialized storage, pubsub beacon, and API clients");

    let mut router = axum::Router::new();

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    if config.services.contains(&facility::Service::WebUI) {
        let auth_backend = auth::WebAuthBackend::new(db.clone());
        router = router.merge(webui::v1::make_ui_router(
            user_api.clone(),
            session_store,
            auth_backend,
            config.cookie_secure,
        ));

        // If using the local Storage provider, we serve media files manually
        if let Some(local_url_prefix) = &config.storage_local_url_prefix {
            if let Some(local_file_path) = &config.storage_local_file_path {
                router = router.nest_service(local_url_prefix, ServeDir::new(local_file_path));
                tracing::info!("Mounted media file serving routes for local storage");
            }
        }
        tracing::info!("Initialized web UI routes");
    }

    // REST API routes (JWT-protected)
    if config.services.contains(&facility::Service::API) {
        let jwt = auth::JwtConfig::from_secret(
            config
                .jwt_secret
                .as_ref()
                .context("JWT_SECRET not set, required for API")?
                .as_bytes(),
        );
        let api_router = rest::make_router(user_api.clone(), jwt);
        router = router.nest("/api", api_router);
        tracing::info!("Initialized REST API routes");
    }

    // Webhook routes (no auth, protected by GCloud IAM in prod)
    if config.services.contains(&facility::Service::Webhook) {
        let illuminator = illumination::make_illuminator(&config, stg.clone());
        router = router.nest("/_wh", webhook::make_router(service_api, illuminator));
        tracing::info!("Initialized webhook routes");
    }

    let host_port = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&host_port)
        .await
        .context("Failed to bind TCP listener")?;
    tracing::info!("Bound listener on {}, will start serving...", host_port);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Receivd Ctrl-C, starting graceful shutdown...");
        })
        .await
        .context("Failed to serve routes")?;

    Ok(())
}
