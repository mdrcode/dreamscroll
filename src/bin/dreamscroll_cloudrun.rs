use anyhow::Context;
use sea_orm::prelude::*;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use dreamscroll::{api, auth, database, facility, model, rest, storage, task, webhook, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // For localdev, we explicity call dotenvy::from_filename() to load:
    //  - .env (secrets, .gitignored for API keys, etc)
    //  - ds_config.env
    //
    // But for production, the configuration should come externally via the
    // environment, set by Google Cloud Secret Manager, etc.
    //
    // To run the Docker container locally, use docker compose (the correct
    // configuration is loaded by the docker-compose.yaml).

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

    let user_count = model::user::Entity::find().count(&db.conn).await?;
    if user_count == 0 {
        tracing::error!(
            "No users found in db! Create a user with `dreamscroll_util check_first_user`."
        );
    } else {
        tracing::info!("Found {} users in database", user_count);
    }

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let task_publisher = task::task_publisher::make_task_publisher(&config);
    let user_api = api::UserApiClient::new(
        db.clone(),
        stg.clone(),
        url_maker.clone(),
        task_publisher.clone(),
    );
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());

    tracing::info!("Initialized storage and API client");

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let auth_backend = auth::WebAuthBackend::new(db.clone());
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);
    tracing::info!("Initialized web UI routes");

    // REST API routes (JWT-protected)
    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);
    tracing::info!("Initialized REST API routes");

    // PubSub Webhook Routes
    let webhook_auth = {
        let verifier = webhook::gcloud::PubSubOidcVerifier::new(
            config
                .pubsub_push_oidc_audience
                .expect("DREAMSCROLL_PUBSUB_PUSH_OIDC_AUDIENCE missing"),
            config.pubsub_push_oidc_service_account_email.clone(),
            config.pubsub_push_oidc_jwks_url.clone(),
        );
        webhook::WebhookAuth::PubSubOidc(verifier)
    };
    router = router.nest(
        "/webhook",
        webhook::make_router(service_api.clone(), stg.clone(), webhook_auth),
    );
    tracing::info!("Initialized pub/sub webhook OIDC verification");

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
