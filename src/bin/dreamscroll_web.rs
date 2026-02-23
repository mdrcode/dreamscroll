use std::time::Duration;

use anyhow::Context;
use axum::http;
use rustls::crypto;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::Span;

use dreamscroll::{
    api, auth, database, facility, illumination, rest, storage, task, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=1 to skip
    // local config files. But we load them when running via `cargo run`
    if std::env::var("NO_LOCAL_CONFIG_FILES").is_err() {
        facility::load_local_config_files();
    }

    facility::init_tracing();
    tracing::info!("Starting dreamscroll_web...");

    let config = facility::make_config()?;

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);
    tracing::info!("Connected to database");

    facility::check_users(&db).await?;

    // task::Beacon is the abstraction by which the app signals that tasks
    // should be enqueued in response to logical events
    let beacon = {
        let pubsub_base_url = task::PubSubBaseUrl::new(
            config.pubsub.project_id.as_str(),
            config.pubsub.emulator_url_base.as_deref(),
        );
        let illumination_queue: Box<dyn task::TopicQueue> = Box::new(task::PubSubTopicQueue::new(
            pubsub_base_url.clone(),
            config.pubsub.illumination_topic_id.as_str(),
        ));
        task::Beacon::builder()
            .illumination_queue(illumination_queue)
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
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);
    tracing::info!("Initialized web UI routes");

    // REST API routes (JWT-protected)
    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);
    tracing::info!("Initialized REST API routes");

    // PubSub Webhook Routes (no auth for localdev emulator, Pub/Sub OIDC for prod)
    let webhook_auth = {
        match config.pubsub.emulator_url_base.as_deref() {
            None => {
                tracing::info!("WebhookAuth: Pub/Sub OIDC verification enabled");
                webhook::WebhookAuth::PubSubOidc(
                    webhook::gcloud::PubSubOidcVerifier::from_config(&config.pubsub)
                        .expect("Failed to create PubSubOidcVerifier"),
                )
            }
            Some(_) => {
                tracing::warn!(
                    "WebhookAuth: PUBSUB_EMULATOR_URL_BASE is set, so webhook auth is DISABLED. Fine for local dev but NOT for prod."
                );
                webhook::WebhookAuth::None
            }
        }
    };
    let illuminator = illumination::make_illuminator("geministructured", stg.clone());
    router = router.nest(
        "/webhook",
        webhook::make_router(webhook_auth, service_api.clone(), illuminator),
    );

    // HTTP tracing (method, status, latency, etc) for all routes
    let router = add_trace_layer(router);

    let host_port = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&host_port)
        .await
        .context("Failed to bind TCP listener")?;
    tracing::info!("Bound listener on {}, will start serving...", host_port);
    let cancel = CancellationToken::new();
    axum::serve(listener, router)
        .with_graceful_shutdown(wait_for_shutdown(cancel.clone()))
        .await
        .context("Failed to serve routes")?;

    Ok(())
}

async fn wait_for_shutdown(cancel: CancellationToken) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received SIGINT (CTRL+C), shutting down...");
            cancel.cancel();
        }
        _ = cancel.cancelled() => {
            tracing::info!("Cancellation requested, shutting down...");
        }
    }
}

fn add_trace_layer(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http()
            .make_span_with(|request: &http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    uri = %request.uri(),
                    http_status = tracing::field::Empty,
                    latency = tracing::field::Empty,
                    error = tracing::field::Empty,
                )
            })
            .on_response(
                |response: &http::Response<_>, latency: Duration, span: &Span| {
                    span.record("http_status", response.status().as_u16());
                    span.record("latency", latency.as_millis() as u64);
                },
            )
            .on_failure(
                |error: tower_http::classify::ServerErrorsFailureClass,
                 latency: Duration,
                 span: &Span| {
                    span.record("latency", latency.as_millis() as u64);
                    span.record("error", format!("{:?}", error));
                },
            ),
    )
}
