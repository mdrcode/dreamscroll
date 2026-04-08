use axum::{Router, extract::DefaultBodyLimit, routing::post};
use std::sync::Arc;

use crate::{api, facility, ignition, illumination, search};

use super::*;

pub struct WebhookState {
    pub service_api: api::ServiceApiClient,
    pub illuminator: Box<dyn illumination::Illuminator>,
    pub firestarter: Box<dyn ignition::Firestarter>,
    pub capture_embedder: search::CaptureEmbedder,
}

pub fn make_webhook_router(
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
    firestarter: Box<dyn ignition::Firestarter>,
    capture_embedder: search::CaptureEmbedder,
) -> Router {
    let state = Arc::new(WebhookState {
        service_api,
        illuminator,
        firestarter,
        capture_embedder,
    });

    // These routes are protected by GCloud IAM/OIDC in production, but have no
    // auth locally since they're only called by the local TaskQueue.
    //
    // This router is nested under "/_wh", so full path will be e.g.
    // "/_wh/cloudtask/illuminate"
    let mut router = Router::new()
        .route("/cloudtask/ingest", post(cloudtask::r_ingest::post))
        .route("/cloudtask/illuminate", post(cloudtask::r_illuminate::post))
        .route(
            "/cloudtask/search_index",
            post(cloudtask::r_search_index::post),
        )
        .route("/cloudtask/spark", post(cloudtask::r_spark::post))
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
