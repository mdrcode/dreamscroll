use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::Tera;
use tower_http::services::ServeDir;
use tower_sessions::SessionManagerLayer;

use crate::{api, auth};

use super::*;

pub struct WebState {
    pub api_client: api::ApiClient,
    pub tera: Tera,
}

pub fn make_ui_router<S>(
    api_client: api::ApiClient,
    session_store: S,
    auth_backend: auth::WebAuthBackend,
) -> Router
where
    S: tower_sessions::SessionStore + Clone,
{
    let session_layer = SessionManagerLayer::new(session_store).with_secure(false); // TODO: Use secure cookies in production

    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    let tera = Tera::new("web/v1/templates/*.tera").expect("Failed to load templates");
    let state = Arc::new(WebState { api_client, tera });

    let mut router = Router::new()
        .route("/login", get(login_page).post(login_handler))
        .route("/logout", get(logout_handler))
        .route(
            "/",
            get(index).layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .route(
            "/search",
            get(search).layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .route(
            "/detail/{capture_id}",
            get(detail).layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .route(
            "/entity/knode/{id}",
            get(entity_knode).layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .route(
            "/entity/social/{id}",
            get(entity_social_media)
                .layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .route(
            "/upload",
            post(upload).layer(login_required!(auth::WebAuthBackend, login_url = "/login")),
        )
        .layer(auth_layer)
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024)) // 5 MB
        .with_state(state);

    // For local dev, we serve static JS/CSS files directly
    router = router.nest_service("/static", ServeDir::new("web/v1/static"));

    router
}
