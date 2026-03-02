use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::Tera;
use tower_http::services::ServeDir;
use tower_sessions::SessionManagerLayer;

use crate::{api, auth, facility};

use super::*;

pub struct WebState {
    pub user_api: api::UserApiClient,
    pub tera: Tera,
}

pub fn make_ui_router(
    user_api: api::UserApiClient,
    session_store: auth::SessionStoreWrapper,
    auth_backend: auth::WebAuthBackend,
    cookie_secure: bool,
) -> Router {
    // Note, this will hang forever if templates fail to load (waiting on iterator)
    let tera = Tera::new("web/v1/templates/*.tera").expect("Failed to load templates");
    tracing::info!("Loaded tera templates");

    let session_layer = SessionManagerLayer::new(session_store)
        // true == only send cookies over HTTPS (production)
        // false == allow cookies over HTTP (local dev)
        .with_secure(cookie_secure)
        // true == JS cannot access cookies
        .with_http_only(true)
        // SameSite::Lax: cookie is sent on top-level GET navigations (links)
        // but NOT on cross-site form POSTs or subresource requests, providing
        // CSRF mitigation without breaking normal browser navigation.
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_name("dreamscroll_session");

    let state = Arc::new(WebState { user_api, tera });

    let routes_open =
        Router::new().route("/login", get(r_login_page::get).post(r_auth::login_post));

    let routes_protected = Router::new()
        .route("/", get(r_index::get))
        .route("/search", get(r_search::get))
        .route("/detail/{capture_id}", get(r_detail::get))
        .route("/entity/knode/{id}", get(r_entity::get_knode))
        .route("/entity/social/{id}", get(r_entity::get_social_media))
        .route("/upload", post(r_upload::post))
        .route("/logout", post(r_auth::logout_post))
        .layer(AuthManagerLayerBuilder::new(auth_backend, session_layer).build())
        .layer(login_required!(auth::WebAuthBackend, login_url = "/login"));

    let mut router = axum::Router::new()
        .merge(routes_protected)
        .merge(routes_open)
        .with_state(state);

    // For local dev, we serve static JS/CSS files directly
    router = router.nest_service("/static", ServeDir::new("web/v1/static"));

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
