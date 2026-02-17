use anyhow::Context;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamscroll::{api, auth, database, facility, rest, storage, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // For localdev, we explicity call dotenvy::from_filename() to load:
    //  - ds_config.env
    //  - ds_secrets.env (.gitignored for API keys, etc)
    // But for production, the configuration should come externally via the
    // environment, set by Google Cloud Secret Manager, etc.
    //
    // When testing the Docker image locally, can run docker with
    //  `--env-file ds_config.env --env-file ds_secrets.env`

    facility::init_tracing();
    let config = facility::make_config();

    // Verify secrets are available
    config
        .jwt_secret
        .as_ref()
        .context("DREAMSCROLL_JWT_SECRET missing")?;

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let user_api = api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone());
    tracing::info!("Initialized storage and API client");

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let auth_backend = auth::WebAuthBackend::new(db.clone());
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);
    tracing::info!("Initialized web UI router");

    // REST API routes (JWT-protected)
    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);
    tracing::info!("Initialized REST API router");

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let t = tokio::spawn(async move {
        let host_port = format!("0.0.0.0:{}", config.port);
        let listener = TcpListener::bind(&host_port).await.unwrap();
        tracing::info!("Bound listener on {}, will start serving...", host_port);
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                cancel_clone.cancelled().await;
            })
            .await
            .expect("Failed to serve web.");
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("Received SIGINT (CTRL+C), shutting down...");
    cancel.cancel();
    let _ = tokio::join!(t);

    Ok(())
}
