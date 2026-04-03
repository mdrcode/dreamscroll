use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
};

use crate::{api, auth, facility};

use super::*;

pub struct RestState {
    pub user_api: api::UserApiClient,
    pub admin_api: api::AdminApiClient,
    pub jwt_config: auth::JwtConfig,
}

/// Creates the API router with all REST endpoints.
///
/// Most routes require JWT authentication. Include a valid JWT token in the
/// `Authorization: Bearer <token>` header.
///
/// The `/token` endpoint is public and used to obtain JWT tokens.
///
/// Note that these routes are at top level within the returned router.
/// Expectation is that they are nested under `/api`, etc in the main
/// router.
pub fn make_api_router(
    user_api: api::UserApiClient,
    service_api: api::ServiceApiClient,
    beacon: crate::task::Beacon,
    jwt_config: auth::JwtConfig,
) -> Router {
    let admin_api = api::AdminApiClient::new(user_api.db.clone(), service_api, beacon);

    let state = Arc::new(RestState {
        user_api,
        admin_api,
        jwt_config: jwt_config.clone(),
    });

    let routes_open = Router::new().route("/token", post(r_token::post));

    let routes_protected = Router::new()
        .route("/admin/backfill/enqueue", post(r_backfill_enqueue::post))
        .route("/admin/users", post(r_create_user::post))
        .route("/captures", get(r_capture::get))
        .route("/captures/{capture_id}", delete(r_capture::delete))
        .route("/captures/{capture_id}/archive", post(r_capture::archive))
        .route(
            "/captures/{capture_id}/unarchive",
            post(r_capture::unarchive),
        )
        .route("/captures/import", post(r_import_capture::post))
        .route("/dummy", get(r_dummy::get))
        .route("/timeline", get(r_timeline::get))
        .layer(auth::JwtAxumLayer::new(jwt_config));

    let mut router = Router::new()
        .merge(routes_protected)
        .merge(routes_open)
        .with_state(state);

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
