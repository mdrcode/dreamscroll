use anyhow::Context;
use tokio::net::TcpListener;

use dreamscroll::{api, auth, database, facility, rest, storage, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // For localdev, in addition ds_config.env we directly load ds_secrets.env
    // But obviously that's not for production, where secrets should come via
    // external service (e.g. Google Cloud Secret Manager) or if testing the
    // Docker image locally, you can run with `--env-file ds_secrets.env`
    dotenvy::from_filename("ds_config.env").ok();

    let config = facility::make_config();
    facility::init_tracing(&config);

    // Verify secrets are available
    config
        .jwt_secret
        .as_ref()
        .context("DREAMSCROLL_JWT_SECRET missing")?;

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let user_api = api::UserApiClient::new(db.clone(), url_maker.clone());

    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());

    let auth_backend = auth::WebAuthBackend::new(db.clone());

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let mut router =
        webui::v1::make_ui_router(user_api.clone(), stg.clone(), session_store, auth_backend);

    // REST API routes (JWT-protected)
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);

    let host_port = format!("0.0.0.0:{}", config.port);
    tokio::spawn(async move {
        let listener = TcpListener::bind(host_port).await.unwrap();
        axum::serve(listener, router)
            .await
            .expect("Failed to serve dreamscroll web.");
    });

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");

    Ok(())
}
