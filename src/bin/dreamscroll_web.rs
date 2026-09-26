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

    // Populates config env vars from local files unless NO_LOCAL_CONFIG_FILES
    config::populate_env_if_test_or_dev();

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
        return Err(anyhow::anyhow!("No services enabled, nothing to do"));
    } else {
        tracing::info!("Starting dreamscroll_web with services: {:?}", cfg.services);
    }

    tracing::info!("Synchronizing DB schemas if necessary...");
    let (db_connection, session_store) = database::connect(&cfg).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    database::check_users(&db).await?;

    let stg = storage::make_provider(&cfg).await;
    let url_maker = storage::UrlMaker::from_config(&cfg)?;
    // Every app instance may produce or consume these user-wide events.
    let notifier = Some(std::sync::Arc::new(sse::PostgresServerEventNotifier::new(
        db.conn.get_postgres_connection_pool().clone(),
    )) as std::sync::Arc<dyn sse::ServerEventNotifier>);
    let task_master = task::make_task_master(&cfg, db.clone(), notifier).await?;
    let searcher = search::CaptureSearcher::from_config(&cfg).await?;

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
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    if cfg.services.contains(&config::Service::WebUI) {
        let server_event_listener =
            sse::ServerEventListener::connect(&database::make_url_from_config(&cfg, None, false))
                .await?;
        let server_events = sse::spawn_local_fanout(server_event_listener, shutdown_rx.clone());
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

        let ui_router = webui::v2::make_ui_router(
            user_api.clone(),
            task_master.clone(),
            server_events.clone(),
            shutdown_rx.clone(),
            auth_backend.clone(),
            session_layer.clone(),
            cfg.max_upload_bytes,
        );

        router = router.merge(ui_router);

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
        let jwt = auth::JwtConfig::from_secret(secret)
            .with_user_expiration_secs(cfg.jwt_user_expiration_secs)
            .with_leeway(cfg.jwt_validation_leeway_secs);

        let api_router = rest::make_api_router(
            user_api.clone(),
            service_api.clone(),
            task_master.clone(),
            jwt,
            cfg.max_upload_bytes,
        );

        router = router.nest("/api", api_router);

        tracing::info!("Initialized REST API routes");
    }

    // Webhook routes (unauthenticated locally but require OIDC in prod)
    if cfg.services.contains(&config::Service::Webhook) {
        let illuminator = illumination::make_illuminator(&cfg, stg.clone());
        let firestarter = ignition::make_firestarter(&cfg)?;
        let embedder = search::gcloud::GeminiEmbedder::from_config(&cfg)?;
        let vector_store = search::gcloud::VertexVectorStore::from_config(&cfg).await?;
        let webhook_oidc = webhook::oidc_from_config(&cfg)?;

        let webhook_router = webhook::make_webhook_router(
            service_api,
            stg,
            illuminator,
            firestarter,
            embedder,
            vector_store,
            task_master.clone(),
            webhook_oidc,
        );

        router = router.nest("/_wh", webhook_router);
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
        .with_graceful_shutdown(shutdown_signal(shutdown_tx))
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
async fn shutdown_signal(shutdown: tokio::sync::watch::Sender<bool>) {
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

    // Close the listener task and open SSE streams as Axum begins graceful
    // shutdown. Sending is harmless when WebUI disabled and no receivers exist
    let _ = shutdown.send(true);
}
