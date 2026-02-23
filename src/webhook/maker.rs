use axum::{Router, routing::post};
use std::sync::Arc;

use crate::{api, illumination};

use super::*;

pub struct WebhookState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components because they have DB access.
    pub service_api: api::ServiceApiClient,
    pub auth: WebhookAuth,
    pub illuminator: Box<dyn illumination::Illuminator>,
}

pub fn make_router(
    auth: WebhookAuth,
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
) -> Router {
    let state = Arc::new(WebhookState {
        service_api: service_api,
        auth,
        illuminator,
    });

    Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .with_state(state)
}
