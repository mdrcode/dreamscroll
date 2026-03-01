use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::{api, auth};

pub mod r_capture;
pub mod r_dummy;
pub mod r_import_capture;

pub mod r_timeline;
pub mod r_token;

pub struct RestState {
    pub user_api: api::UserApiClient,
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
pub fn make_router(user_api: api::UserApiClient, jwt_config: auth::JwtConfig) -> Router {
    let state = Arc::new(RestState {
        user_api,
        jwt_config: jwt_config.clone(),
    });

    // Public routes (no authentication required)
    let public_routes = Router::new().route("/token", post(r_token::post));

    // Routes that require JWT authentication
    let protected_routes = Router::new()
        .route("/captures", get(r_capture::get))
        .route("/captures/import", post(r_import_capture::post))
        .route("/dummy", get(r_dummy::get))
        .route("/timeline", get(r_timeline::get))
        .layer(auth::JwtAxumLayer::new(jwt_config));

    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .with_state(state)
}
