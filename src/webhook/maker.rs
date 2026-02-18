use std::sync::Arc;

use axum::{Router, routing::post};

use crate::{auth, task};

use super::*;

pub struct InternalRestState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components.
    pub processor: task::processor::CaptureIlluminationProcessor,
    pub webhook_auth: gcloud_pubsub::InternalWebhookAuth,
}

pub fn make_internal_router(
    processor: task::processor::CaptureIlluminationProcessor,
    webhook_auth: gcloud_pubsub::InternalWebhookAuth,
) -> Router {
    let state = Arc::new(InternalRestState {
        processor,
        webhook_auth,
    });

    Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .with_state(state)
}
