use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    api,
    auth::{JwtAxumLayer, JwtConfig, PubSubOidcVerifier},
};

pub mod r_capture;
pub mod r_dummy;
pub mod r_pubsub;
pub mod r_timeline;
pub mod r_token;

pub struct RestState {
    pub user_api: api::UserApiClient,
    pub jwt_config: JwtConfig,
}

pub struct InternalRestState {
    // This processor is intentionally backed by ServiceApiClient and therefore
    // does not require user auth/JWT context. Internal background services are
    // treated as elevated trusted components.
    pub processor: crate::illumination::CaptureIlluminationProcessor,
    pub webhook_auth: InternalWebhookAuth,
}

#[derive(Clone)]
pub enum InternalWebhookAuth {
    None,
    BearerToken(String),
    PubSubOidc(std::sync::Arc<PubSubOidcVerifier>),
}

/// Creates the API router with all REST endpoints.
///
/// Most routes require JWT authentication. Include a valid JWT token in the
/// `Authorization: Bearer <token>` header.
///
/// The `/token` endpoint is public and used to obtain JWT tokens.
pub fn make_api_router(user_api: api::UserApiClient, jwt_config: JwtConfig) -> Router {
    let state = Arc::new(RestState {
        user_api,
        jwt_config: jwt_config.clone(),
    });

    // Routes that require JWT authentication
    let protected_routes = Router::new()
        .route("/captures", get(r_capture::get))
        .route("/dummy", get(r_dummy::get))
        .route("/timeline", get(r_timeline::get))
        .layer(JwtAxumLayer::new(jwt_config));

    // Public routes (no authentication required)
    let public_routes = Router::new().route("/token", post(r_token::post));

    Router::new()
        .merge(protected_routes)
        .merge(public_routes)
        .with_state(state)
}

pub fn make_internal_router(
    processor: crate::illumination::CaptureIlluminationProcessor,
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
