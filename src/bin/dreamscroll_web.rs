use anyhow::Context;
use rustls::crypto;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, SessionManagerLayer, cookie};

use dreamscroll::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG=(any value).
    // But when running via `cargo run` we load local files as a convenience.
    if std::env::var("NO_LOCAL_CONFIG").is_err() {
        config::load_local_files();
    }

    let trace_provider = {
        if std::env::var("K_SERVICE").is_ok() {
            // Running within Cloud Run, so enable Cloud Trace/Logging with
            // integrated trace/span IDs and Cloud Logging JSON formatting.
            let project_id = std::env::var("GCLOUD_PROJECT_ID")
                .context("GCLOUD_PROJECT_ID env var required but not set")?;
            Some(telemetry::init_gcloud(project_id).await?)
        } else {
            telemetry::init_local();
            None
        }
    };

    let cfg = config::make()?;

    if cfg.services.is_empty() {
        tracing::warn!("No services enabled (set SERVICES env var to enable)");
        return Err(anyhow::anyhow!("No services enabled, nothing to do"));
    } else {
        tracing::info!("Starting dreamscroll_web with services: {:?}", cfg.services);
    }

    let (db_connection, session_store) = database::connect(&cfg).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    database::check_users(&db).await?;

    let stg = storage::make_provider(&cfg).await;
    let url_maker = storage::UrlMaker::from_config(&cfg);
    let task_master = task::make_task_master(&cfg, db.clone()).await?;
    let searcher = search::CaptureSearcher::from_config(&cfg)
        .await
        .context("Failed to initialize required CaptureSearcher")?;

    let user_api = api::UserApiClient::new(
        db.clone(),
        stg.clone(),
        url_maker.clone(),
        task_master.clone(),
        searcher,
    );
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    tracing::info!("Initialized storage, task master, and API clients");

    let mut router = axum::Router::new();

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    if cfg.services.contains(&config::Service::WebUI) {
        let auth_backend = auth::WebAuthBackend::new(db.clone());

        let session_layer = SessionManagerLayer::new(session_store)
            // Refresh session on read, not just modify (to extend inactivity timeout)
            .with_always_save(cfg.session_always_save)
            // Expire session after seven days of inactivity
            .with_expiry(Expiry::OnInactivity(cookie::time::Duration::days(7)))
            // true == only send cookies over HTTPS (production)
            // false == allow cookies over HTTP (local dev)
            .with_secure(cfg.cookie_secure)
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

        // If using the local Storage provider, we serve media files manually
        if let Some(local_url_prefix) = &cfg.storage_local_url_prefix
            && let Some(local_file_path) = &cfg.storage_local_file_path
        {
            router = router.nest_service(local_url_prefix, ServeDir::new(local_file_path));
            tracing::info!("Mounted media file serving routes for local storage");
        }
        tracing::info!("Initialized web UI routes");
    }

    // REST API routes (JWT-protected)
    if cfg.services.contains(&config::Service::API) {
        let secret = cfg
            .jwt_secret
            .as_ref()
            .context("JWT_SECRET not set, required for API")?
            .as_bytes();
        let jwt = auth::JwtConfig::from_secret(secret);
        router = router.nest(
            "/api",
            rest::make_api_router(user_api.clone(), service_api.clone(), task_master.clone(), jwt),
        );
        tracing::info!("Initialized REST API routes");
    }

    // Webhook routes (no auth locally, protected by GCloud IAM/OIDC in prod)
    if cfg.services.contains(&config::Service::Webhook) {
        let illuminator = illumination::make_illuminator(&cfg, stg.clone());
        let firestarter = ignition::make_firestarter(&cfg)?;
        let embedder = search::gcloud::GeminiEmbedder::from_config(&cfg)?;
        let vector_store = search::gcloud::VertexVectorStore::from_config(&cfg).await?;
        router = router.nest(
            "/_wh",
            webhook::make_webhook_router(
                service_api,
                stg,
                illuminator,
                firestarter,
                embedder,
                vector_store,
            ),
        );
        tracing::info!("Initialized webhook routes");
    }

    let host_port = format!("0.0.0.0:{}", cfg.port);
    let listener = TcpListener::bind(&host_port)
        .await
        .context("Failed to bind TCP listener")?;
    tracing::info!(
        "Bound listener on {}, will start serving {:?}...",
        host_port,
        cfg.services
    );
    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    if let Some(provider) = trace_provider {
        tracing::info!("Flushing Cloud Trace spans before shutdown...");
        if let Err(error) = provider.shutdown() {
            tracing::error!(error = %error, "Failed to flush Cloud Trace spans");
        }
    }

    serve_result.context("Axum failed to serve routes")?;

    Ok(())
}

// Cloud Run sends SIGTERM, so simply relying on tokio's ctrl_c() is inadequate
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received Ctrl-C, starting graceful shutdown...");
            }
            _ = terminate.recv() => {
                tracing::info!("Received SIGTERM, starting graceful shutdown...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("Received shutdown signal, starting graceful shutdown...");
    }
}
