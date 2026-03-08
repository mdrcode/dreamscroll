use axum::{Router, extract::DefaultBodyLimit, routing::post};
use std::sync::Arc;

use crate::{api, facility, illumination};

use super::*;

pub struct WebhookState {
    pub service_api: api::ServiceApiClient,
    pub illuminator: Box<dyn illumination::Illuminator>,
}

pub fn make_webhook_router(
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
) -> Router {
    tracing::warn!(
        "Initializing webhook routes with NO AUTH requirement. \
        Prod auth should be enforced by Google Cloud IAM and/or API Gateway."
    );

    let state = Arc::new(WebhookState {
        service_api,
        illuminator,
    });

    let mut router = Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .route(
            "/illumination/cloudtask",
            post(r_wh_illuminate_cloudtask::post),
        )
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
