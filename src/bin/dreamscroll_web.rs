use anyhow::Context;
use rustls::crypto;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;

use dreamscroll::webhook::logic::{illuminate::IlluminationTask, spark::SparkTask};
use dreamscroll::{
    api, auth, database, facility, illumination, rest, storage, task, webhook, webui,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    crypto::CryptoProvider::install_default(crypto::aws_lc_rs::default_provider())
        .expect("Failed to install aws_lc_rs as default crypto provider");

    // Containerized environments should set NO_LOCAL_CONFIG_FILES=(any value).
    // But when running via `cargo run` we load local files for convenience.
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
    let beacon = make_beacon(&config).await?;

    let user_api = api::UserApiClient::new(db.clone(), stg.clone(), url_maker.clone(), beacon);
    let service_api = api::ServiceApiClient::new(db.clone(), url_maker.clone());
    tracing::info!("Initialized storage, pubsub beacon, and API clients");

    let mut router = axum::Router::new();

    // Web UI routes (Session-auth protected) + static JS/CSS serving
    if config.services.contains(&facility::Service::WebUI) {
        let auth_backend = auth::WebAuthBackend::new(db.clone());
        router = router.merge(webui::v1::make_ui_router(
            user_api.clone(),
            session_store,
            auth_backend,
            config.cookie_secure,
        ));

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
        router = router.nest("/api", rest::make_api_router(user_api.clone(), jwt));
        tracing::info!("Initialized REST API routes");
    }

    // Webhook routes (no auth locally, protected by GCloud IAM/OIDC in prod)
    if config.services.contains(&facility::Service::Webhook) {
        let illuminator = illumination::make_illuminator(&config, stg.clone());
        let firestarter = dreamscroll::ignition::make_firestarter(&config)?;
        router = router.nest(
            "/_wh",
            webhook::make_webhook_router(service_api, illuminator, firestarter),
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

async fn make_beacon(config: &facility::Config) -> anyhow::Result<task::Beacon> {
    match config.task_backend {
        task::TaskQueueBackend::Local => {
            let illumination_queue =
                task::LocalTaskQueue::connect(4, |_task: IlluminationTask| async move { Ok(()) });
            let spark_queue =
                task::LocalTaskQueue::connect(4, |_task: SparkTask| async move { Ok(()) });

            Ok(task::Beacon::builder()
                .illumination_queue(illumination_queue)
                .spark_queue(spark_queue)
                .build())
        }
        task::TaskQueueBackend::GCloudPubSub => {
            let emulator = config.task_pubsub_emulator.as_deref();
            let illumination_queue = task::PubSubTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config
                    .task_pubsub_topic_new_capture
                    .as_ref()
                    .expect("TASK_PUBSUB_TOPIC_NEW_CAPTURE not set"),
                emulator,
            )
            .await
            .context("Failed to initialize Pub/Sub queue: Illumination")?;
            let spark_queue = task::PubSubTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config
                    .task_pubsub_topic_spark
                    .as_ref()
                    .expect("TASK_PUBSUB_TOPIC_SPARK not set"),
                emulator,
            )
            .await
            .context("Failed to initialize Pub/Sub queue: Spark")?;

            Ok(task::Beacon::builder()
                .illumination_queue(illumination_queue)
                .spark_queue(spark_queue)
                .build())
        }
        task::TaskQueueBackend::GCloudTasks => {
            let illumination_queue = task::CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_illumination
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_ILLUMINATION not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Illumination")?;
            let spark_queue = task::CloudTaskQueue::connect(
                config.gcloud_project_id.as_str(),
                config.gcloud_project_region.as_str(),
                config
                    .task_cloudtask_queue_spark
                    .as_ref()
                    .expect("TASK_CLOUDTASK_QUEUE_SPARK not set"),
            )
            .await
            .context("Failed to initialize Cloud Tasks Queue: Spark")?;

            Ok(task::Beacon::builder()
                .illumination_queue(illumination_queue)
                .spark_queue(spark_queue)
                .build())
        }
    }
}
