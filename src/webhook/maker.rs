use axum::{Router, extract::DefaultBodyLimit, routing::post};
use std::sync::Arc;

use crate::{api, ignition, illumination, search, storage, task, telemetry};

use super::*;

pub fn make_webhook_router(
    service_api: api::ServiceApiClient,
    storage: Box<dyn storage::StorageProvider>,
    illuminator: Box<dyn illumination::Illuminator>,
    firestarter: Box<dyn ignition::Firestarter>,
    embedder: search::gcloud::GeminiEmbedder,
    vector_store: search::gcloud::VertexVectorStore,
    task_master: Arc<task::TaskMaster>,
) -> Router {
    let state = Arc::new(WebhookState {
        service_api,
        stg: storage,
        illuminator,
        firestarter,
        embedder,
        vector_store,
        task_master,
    });

    // These routes are protected by GCloud IAM/OIDC in production, but have no
    // auth locally since they're only called by the local TaskQueue.
    //
    // This router is nested under "/_wh", so full path will be e.g.
    // "/_wh/cloudtask/illuminate"
    let mut router = Router::new()
        .route("/cloudtask/ingest", post(r_ingest::post))
        .route("/cloudtask/illuminate", post(r_illuminate::post))
        .route("/cloudtask/search_index", post(r_search_index::post))
        .route("/cloudtask/spark", post(r_spark::post))
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = telemetry::add_axum_trace_propagation(router); // Cloud Run trace headers
    router
}
