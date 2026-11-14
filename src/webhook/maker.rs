use axum::{Router, extract::DefaultBodyLimit, middleware, routing::post};
use std::sync::Arc;

use crate::telemetry;

use super::*;

/// One Cloud Tasks delivery — not an upload (Cloud Tasks caps a task at 10 MB).
const TASK_PAYLOAD_LIMIT: usize = 5 * 1024 * 1024;

/// The queue names this router serves, and the only names the app may be configured with.
///
/// The suffix is the queue name below `/_wh/cloudtask/`: Cloud Tasks delivers to the URL built from
/// `Config::task_queue_name_*`, so a configured name with no route is a 404 per delivery. Boot
/// validation ([`validate_queue_names`]) turns that into a startup error.
const QUEUE_ILLUMINATE: &str = "illuminate";
const QUEUE_SEARCH_INDEX: &str = "search-index";
const QUEUE_SPARK: &str = "spark";
const QUEUE_KNOWLEDGE_LINK: &str = "knowledge-link";
const QUEUE_IDENTITY_RESOLVE: &str = "identity-resolve";

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
        .route(
            &format!("/cloudtask/{QUEUE_ILLUMINATE}"),
            post(r_illuminate::post),
        )
        .route(
            &format!("/cloudtask/{QUEUE_SEARCH_INDEX}"),
            post(r_search_index::post),
        )
        .route(&format!("/cloudtask/{QUEUE_SPARK}"), post(r_spark::post))
        .route(
            &format!("/cloudtask/{QUEUE_KNOWLEDGE_LINK}"),
            post(r_knowledge_link::post),
        )
        .route(
            &format!("/cloudtask/{QUEUE_IDENTITY_RESOLVE}"),
            post(r_identity_resolve::post),
        )
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(TASK_PAYLOAD_LIMIT));
    // Local queues omit this layer; Cloud Tasks always supplies it.
    if let Some(oidc_verifier) = oidc {
        router = router.layer(middleware::from_fn(move |request, next| {
            require_google_id_token(oidc_verifier.clone(), request, next)
        }));
    }
    router = telemetry::add_axum_trace_propagation(router); // Cloud Run trace headers
    router
}

/// Fail boot when a configured queue name has no route. `Config` knows the names; this module knows
/// the routes, so the check belongs where both are visible.
pub fn validate_queue_names<'a>(names: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
    let served = [
        QUEUE_ILLUMINATE,
        QUEUE_SEARCH_INDEX,
        QUEUE_SPARK,
        QUEUE_KNOWLEDGE_LINK,
        QUEUE_IDENTITY_RESOLVE,
    ];
    for name in names {
        anyhow::ensure!(
            served.contains(&name),
            "task queue `{name}` has no webhook route; served: {served:?}"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_queue_must_have_a_route() {
        assert!(validate_queue_names([QUEUE_ILLUMINATE, QUEUE_IDENTITY_RESOLVE]).is_ok());
        let error = validate_queue_names(["spark-typo"]).expect_err("unknown queue is refused");
        assert!(error.to_string().contains("spark-typo"), "{error}");
        assert!(error.to_string().contains("identity-resolve"), "{error}");
    }
}
