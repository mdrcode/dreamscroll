use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use dreamscroll::{api, auth, database, facility, illumination, rest, storage, task, webui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::from_filename("ds_local_config.env").ok();
    let _ = dotenvy::from_filename(".env"); // gitignored for api keys

    facility::init_tracing();
    let config = facility::make_config();

    let (db_connection, session_store) = database::connect(&config).await?;
    let db = database::DbHandle::new(db_connection);

    facility::check_first_user(&db).await?;

    let stg = storage::make_provider(&config).await;
    let url_maker = storage::UrlMaker::new(&config);
    let task_publisher = task::task_publisher::make_task_publisher(&config);

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    let user_api = api::UserApiClient::new(
        db.clone(),
        stg.clone(),
        url_maker.clone(),
        task_publisher.clone(),
    );
    let auth_backend = auth::WebAuthBackend::new(db.clone());
    let mut router = webui::v1::make_ui_router(user_api.clone(), session_store, auth_backend);

    // Web serving for media assets stored with the local storage provider
    router = router.nest_service(
        &config.storage_local_url_prefix.unwrap(),
        ServeDir::new(&config.storage_local_file_path.unwrap()),
    );

    // REST API routes (JWT-protected)
    let jwt_secret = config.jwt_secret.expect("DREAMSCROLL_JWT_SECRET missing");
    let jwt = auth::JwtConfig::from_secret(jwt_secret.as_bytes());
    let api_router = rest::make_api_router(user_api.clone(), jwt.clone());
    router = router.nest("/api", api_router);

    // PubSub Webhook Routes
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    let processor = task::processor::CaptureIlluminationProcessor::new(
        service_api.clone(),
        illumination::make_illuminator("geministructured", stg.clone()),
    );
    router = router.nest(
        "/internal",
        task::webhook::make_internal_router(
            processor.clone(),
            task::webhook::InternalWebhookAuth::None,
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
        let worker = task::orchestrator::make_worker(service_api, gemini);
        let cancel = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = worker.run() => {}
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
