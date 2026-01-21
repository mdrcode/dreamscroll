use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    auth::{JwtConfig, JwtLayer},
    database::DbHandle,
};

pub mod r_capture;
pub mod r_dummy;
pub mod r_timeline;
pub mod r_token;

pub struct ApiState {
    pub db: Arc<DbHandle>,
    pub jwt_config: Arc<JwtConfig>,
}

/// Creates the API router with all REST endpoints.
///
/// Most routes require JWT authentication. Include a valid JWT token in the
/// `Authorization: Bearer <token>` header.
///
/// The `/token` endpoint is public and used to obtain JWT tokens.
pub fn make_api_router(db: Arc<DbHandle>, jwt_config: Arc<JwtConfig>) -> Router {
    let state = Arc::new(ApiState {
        db,
        jwt_config: jwt_config.clone(),
    });

    // Routes that require JWT authentication
    let protected_routes = Router::new()
        .route("/capture/{id}", get(r_capture::get))
        .route("/dummy", get(r_dummy::get))
        .route("/timeline", get(r_timeline::get))
        .layer(JwtLayer::new(jwt_config));

    // Public routes (no authentication required)
    let public_routes = Router::new().route("/token", post(r_token::post));

    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .with_state(state)
}
