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

    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    // Note, this will hang forever if templates fail to load (waiting on iterator)
    let tera = Tera::new("web/v1/templates/*.tera").expect("Failed to load templates");
    tracing::info!("Loaded tera templates");

    let state = Arc::new(WebState { user_api, tera });

    let mut router = Router::new()
        .route("/login", get(login_page).post(login_handler))
        // logout is POST-only: prevents forced-logout via navigation/redirect CSRF.
        .route("/logout", post(logout_handler))
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
        .with_state(state);

    // For local dev, we serve static JS/CSS files directly
    router = router.nest_service("/static", ServeDir::new("web/v1/static"));
    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
