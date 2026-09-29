use axum::{Router, extract::DefaultBodyLimit, middleware, routing::post};
use std::sync::Arc;

use crate::telemetry;

use super::*;

pub fn make_webhook_router(
    state: WebhookState,
    oidc: Option<Arc<google_cloud_auth::credentials::idtoken::verifier::Verifier>>,
) -> Router {
    let state = Arc::new(state);
    // These routes are protected by GCloud OIDC in production, but have no
    // auth locally since they're only called by the local TaskQueue.
    //
    // This router is nested under "/_wh", so full path will be e.g.
    // "/_wh/cloudtask/illuminate"
    //
    // Route suffixes intentionally match the configured Cloud Tasks queue names.
    let mut router = Router::new()
        .route("/cloudtask/illuminate", post(r_illuminate::post))
        .route("/cloudtask/search-index", post(r_search_index::post))
        .route("/cloudtask/spark", post(r_spark::post))
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    // Local queues omit this layer; Cloud Tasks always supplies it.
    if let Some(oidc_verifier) = oidc {
        router = router.layer(middleware::from_fn(move |request, next| {
            require_google_id_token(oidc_verifier.clone(), request, next)
        }));
    }
    router = telemetry::add_axum_trace_propagation(router); // Cloud Run trace headers
    router
}
