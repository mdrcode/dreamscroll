use axum::{Router, extract::DefaultBodyLimit, routing::post};
use std::sync::Arc;

use crate::{api, facility, ignition, illumination};

use super::*;

pub struct WebhookState {
    pub service_api: api::ServiceApiClient,
    pub illuminator: Box<dyn illumination::Illuminator>,
    pub firestarter: Box<dyn ignition::Firestarter>,
}

pub fn make_webhook_router(
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
    firestarter: Box<dyn ignition::Firestarter>,
) -> Router {
    let state = Arc::new(WebhookState {
        service_api,
        illuminator,
        firestarter,
    });

    let mut router = Router::new()
        .route("/pubsub/illuminate", post(pubsub::r_illuminate::post))
        .route("/cloudtask/illuminate", post(cloudtask::r_illuminate::post))
        .route("/cloudtask/spark", post(cloudtask::r_spark::post))
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
