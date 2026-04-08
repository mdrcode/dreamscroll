use anyhow::Context;
use rustls::crypto;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, SessionManagerLayer, cookie};

use dreamscroll::{
    api, auth, database, facility, illumination, rest, search, storage, task, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=(any value).
    // But when running via `cargo run` we load local files as a convenience.
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
    let beacon = task::make_beacon(&config).await?;
    let searcher = search::CaptureSearcher::from_config(&config)
        .await
        .context("Failed to initialize required CaptureSearcher")?;

    let user_api = api::UserApiClient::new(
        db.clone(),
        stg.clone(),
        url_maker.clone(),
        beacon.clone(),
        searcher,
    );
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    tracing::info!("Initialized storage, pubsub beacon, and API clients");

    let mut router = axum::Router::new();

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    if config.services.contains(&facility::Service::WebUI) {
        let auth_backend = auth::WebAuthBackend::new(db.clone());

        let session_layer = SessionManagerLayer::new(session_store)
            // Refresh session on read, not just modify (to extend inactivity timeout)
            .with_always_save(config.session_always_save)
            // Expire session after seven days of inactivity
            .with_expiry(Expiry::OnInactivity(cookie::time::Duration::days(7)))
            // true == only send cookies over HTTPS (production)
            // false == allow cookies over HTTP (local dev)
            .with_secure(config.cookie_secure)
            // true == JS cannot access cookies
            .with_http_only(true)
            // SameSite::Lax: cookie is sent on top-level GET navigations (links)
            // but NOT on cross-site form POSTs or subresource requests, providing
            // CSRF mitigation without breaking normal browser navigation.
            .with_same_site(tower_sessions::cookie::SameSite::Lax)
            .with_name("dreamscroll_session");

        router = router.merge(webui::v2::make_ui_router(
            user_api.clone(),
            auth_backend.clone(),
            session_layer.clone(),
        ));
        router = router.nest(
            "/v1",
            webui::v1::make_ui_router(user_api.clone(), auth_backend, session_layer),
        );

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
        let secret = config
            .jwt_secret
            .as_ref()
            .context("JWT_SECRET not set, required for API")?
            .as_bytes();
        let jwt = auth::JwtConfig::from_secret(secret);
        router = router.nest(
            "/api",
            rest::make_api_router(user_api.clone(), service_api.clone(), beacon.clone(), jwt),
        );
        tracing::info!("Initialized REST API routes");
    }

    // Webhook routes (no auth locally, protected by GCloud IAM/OIDC in prod)
    if config.services.contains(&facility::Service::Webhook) {
        let illuminator = illumination::make_illuminator(&config, stg.clone());
        let firestarter = dreamscroll::ignition::make_firestarter(&config)?;
        let embedder = search::CaptureEmbedder::from_config(&config, stg.clone())
            .await
            .context("Failed to initialize webhook CaptureEmbedder")?;
        router = router.nest(
            "/_wh",
            webhook::make_webhook_router(service_api, illuminator, firestarter, embedder),
        );
        tracing::info!("Initialized webhook routes");
    }

    let host_port = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&host_port)
        .await
        .context("Failed to bind TCP listener")?;
    tracing::info!(
        "Bound listener on {}, will start serving {:?}...",
        host_port,
        config.services
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Receivd Ctrl-C, starting graceful shutdown...");
        })
        .await
        .context("Failed to serve routes")?;

    Ok(())
}
