use axum::{Router, routing::post};
use std::sync::Arc;

use crate::{api, illumination};

use super::*;

pub struct WebhookState {
    pub auth: WebhookAuth,
    pub service_api: api::ServiceApiClient,
    pub illuminator: Box<dyn illumination::Illuminator>,
}

pub fn make_router(
    auth: WebhookAuth,
    service_api: api::ServiceApiClient,
    illuminator: Box<dyn illumination::Illuminator>,
) -> Router {
    let state = Arc::new(WebhookState {
        auth,
        service_api,
        illuminator,
    });

    tracing::warn!(
        "Initializing webhook routes with NO AUTH requirement. \
        Prod auth should be enforced by Google Cloud IAM and/or API Gateway."
    );

    Router::new()
        .route("/illumination/push", post(r_wh_illuminate::post))
        .with_state(state)
}
