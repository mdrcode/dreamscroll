use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::{api, auth, illumination::Illuminator, task};

use super::*;

#[derive(Clone)]
pub enum InternalWebhookAuth {
    None,
    BearerToken(String),
    PubSubOidc(std::sync::Arc<auth::PubSubOidcVerifier>),
}

pub struct InternalRestState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components.
    pub processor: task::processor::CaptureIlluminationProcessor,
    pub webhook_auth: InternalWebhookAuth,
}

pub fn make_internal_router(
    processor: task::processor::CaptureIlluminationProcessor,
    webhook_auth: InternalWebhookAuth,
) -> Router {
    let state = Arc::new(InternalRestState {
        processor,
        webhook_auth,
    });

    Router::new()
        .route("/illumination/push", post(r_pubsub::post))
        .with_state(state)
}
