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

    tracing::info!("Starting dreamscroll_web...");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=(any value).
    // But when running via `cargo run` we load local files as a convenience.
    if !std::env::var("NO_LOCAL_CONFIG_FILES").is_ok() {
        facility::load_local_config_files();
    }

    facility::init_tracing().await?;
    let config = facility::make_config()?;

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    facility::check_users(&db).await?;

    // pubsub::Beacon is the abstraction by which the app signals that tasks
    // should be enqueued in response to logical events
    let beacon = {
        let base_url = pubsub::gcloud::PubSubBaseUrl::new(
            config.project_id.as_str(),
            config.pubsub_emulator_base_url.as_deref(),
        );
        let new_capture_topic = pubsub::gcloud::PubSubTopicQueue::new(
            &base_url,
            config.pubsub_topic_id_new_capture.as_str(),
        );
        pubsub::Beacon::builder()
            .new_capture_topic(Box::new(new_capture_topic) as Box<dyn pubsub::TopicQueue>)
            .build()
    };

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let user_api =
        api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), beacon.clone());
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    tracing::info!("Initialized storage and API clients");

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let auth_backend = auth::WebAuthBackend::new(db.clone());
    let mut router = webui::v1::make_ui_router(
        user_api.clone(),
        session_store,
        auth_backend,
        config.cookie_secure,
    );
    tracing::info!("Initialized web UI routes");

    // If using the local Storage provider, we must serve media files manually
    if config.storage_backend == storage::StorageBackend::Local {
        router = router.nest_service(
            &config.storage_local_url_prefix.clone().unwrap(),
            ServeDir::new(&config.storage_local_file_path.clone().unwrap()),
        );
        tracing::info!("Mounted media file serving routes for local storage");
    }

    // REST API routes (JWT-protected)
    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);
    tracing::info!("Initialized REST API routes");

    // Webhook routes (no auth, protected by GCloud IAM in prod)
    let illuminator = illumination::make_illuminator(&config, "geministructured", stg.clone());
    router = router.nest(
        "/webhook",
        webhook::make_router(webhook::WebhookAuth::None, service_api.clone(), illuminator),
    );

    // Propagates Cloud Run's trace headers to that our logs are correlated properly
    let router = facility::add_trace_propagation_layer(router);

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
