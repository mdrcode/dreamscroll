use std::time::Duration;

use axum::http;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::Span;

use dreamscroll::{
    api, auth, database, facility, illumination, rest, storage, task, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::from_filename("ds_local_config.env");
    let _ = dotenvy::from_filename(".env"); // gitignored for api keys

    facility::init_tracing();
    let config = facility::make_config();

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    facility::check_first_user(&db).await?;

    // task::Beacon is the abstraction by which the app signals that tasks
    // should be enqueued in response to logical events
    let beacon = {
        let pubsub_config = &config.pubsub;
        let pubsub_base_url = task::PubSubHttpBaseUrl::from_config(pubsub_config);
        let illumination_queue: Box<dyn task::TopicQueue> =
            Box::new(task::PubSubHttpTaskQueue::new(
                pubsub_base_url.clone(),
                pubsub_config.illumination_topic_id.as_str(),
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

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let auth_backend = auth::WebAuthBackend::new(db.clone());
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);

    // Web serving for media assets stored with the local storage provider
    router = router.nest_service(
        &config.storage_local_url_prefix.unwrap(),
        ServeDir::new(&config.storage_local_file_path.unwrap()),
    );

    // REST API routes (JWT-protected)
    let jwt_secret = config.jwt_secret.expect("JWT_SECRET missing");
    let jwt = auth::JwtConfig::from_secret(jwt_secret.as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);

    // PubSub Webhook Routes (no auth for localdev)
    router = router.nest(
        "/webhook",
        webhook::make_router(service_api.clone(), stg.clone(), webhook::WebhookAuth::None),
    );
    tracing::warn!("Initialized pub/sub webhook routes with NO AUTH (for local development only)");

    // Tracing layer for all routes
    let router = router.layer(
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
    );

    let cancel_token = CancellationToken::new();
    let cancel = cancel_token.clone();
    let host_port = format!("0.0.0.0:{}", config.port);
    let thread_web = tokio::spawn(async move {
        let listener = TcpListener::bind(host_port).await.unwrap();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                cancel.cancelled().await;
            })
            .await
            .expect("Failed to serve Web.");
    });
    println!("Web UI serving locally at http://localhost:{}", config.port);

    let thread_illuminator = {
        let gemini = illumination::make_illuminator("geministructured", stg.clone());
        let _worker = task::orchestrator::make_worker(service_api, gemini);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                //_ = worker.run() => {}
                _ = cancel.cancelled() => {}
            }
        })
    };

    let _ = webbrowser::open(&format!("http://localhost:{}", config.port));

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    cancel_token.cancel();
    let _ = tokio::join!(thread_web, thread_illuminator);

    Ok(())
}
