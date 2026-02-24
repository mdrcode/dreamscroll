use anyhow::Context;
use axum::http;
use rustls::crypto;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use dreamscroll::{
    api, auth, database, facility, illumination, pubsub, rest, storage, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=1 to skip.
    // But when running via `cargo run` we load local files as a convenience.
    if std::env::var("NO_LOCAL_CONFIG_FILES").is_err() {
        facility::load_local_config_files();
    }

    facility::init_tracing().await?;
    tracing::info!("Starting dreamscroll_web...");

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
            config.pubsub.emulator_base_url.as_deref(),
        );
        let new_capture_topic = pubsub::gcloud::PubSubTopicQueue::new(
            &base_url,
            config.pubsub.topic_id_new_capture.as_str(),
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
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);
    tracing::info!("Initialized web UI routes");

    // If using the local Storage provider, we must serve media files manually
    if config.storage_backend == storage::StorageBackend::Local {
        router = router.nest_service(
            &config.storage_local_url_prefix.unwrap(),
            ServeDir::new(&config.storage_local_file_path.unwrap()),
        );
        tracing::info!("Mounted media file serving routes for local storage");
    }

    // REST API routes (JWT-protected)
    let jwt = auth::JwtConfig::from_secret(config.jwt_secret.as_ref().unwrap().as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);
    tracing::info!("Initialized REST API routes");

    // PubSub Webhook Routes (no auth for localdev emulator, Pub/Sub OIDC for prod)
    let webhook_auth = {
        match config.pubsub.emulator_base_url.as_deref() {
            None => {
                tracing::info!("WebhookAuth: Pub/Sub OIDC verification enabled");
                webhook::WebhookAuth::PubSubOidc(
                    pubsub::gcloud::OidcVerifier::from_config(&config.pubsub)
                        .expect("Failed to create PubSubOidcVerifier"),
                )
            }
            Some(_) => {
                tracing::warn!(
                    "WebhookAuth: PUBSUB_EMULATOR_BASE_URL is set, so webhook auth is DISABLED. Fine for local dev but NOT for prod."
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

struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

fn add_trace_layer(router: axum::Router) -> axum::Router {
    router.layer(
        TraceLayer::new_for_http().make_span_with(|request: &http::Request<_>| {
            // Extract the W3C traceparent header injected by Cloud Run so that
            // our spans are children of the infrastructure-level request trace.
            let parent_cx = opentelemetry::global::get_text_map_propagator(|prop| {
                prop.extract(&HeaderExtractor(request.headers()))
            });
            let span = tracing::info_span!("http_request");

            let _ = span.set_parent(parent_cx);
            span
        }),
    )
}
