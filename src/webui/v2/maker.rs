use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::{Context, Tera};
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, SessionManagerLayer, cookie};

use crate::{api, auth, facility};

use super::{r_auth, r_cards, r_command, r_index, r_login_page, r_upload};

pub struct WebState {
    pub user_api: api::UserApiClient,
    pub tera: Tera,
    pub static_asset_version: String,
}

impl WebState {
    pub fn template_context(&self) -> Context {
        let mut context = Context::new();
        context.insert("static_asset_version", &self.static_asset_version);
        context
    }
}

pub fn make_ui_router(
    user_api: api::UserApiClient,
    session_store: auth::SessionStoreWrapper,
    auth_backend: auth::WebAuthBackend,
    cookie_secure: bool,
) -> Router {
    let tera = Tera::new("web/v2/templates/**/*.tera").expect("Failed to load v2 templates");
    tracing::info!("Loaded v2 tera templates");

    let session_layer = SessionManagerLayer::new(session_store)
        .with_expiry(Expiry::OnInactivity(cookie::time::Duration::days(2)))
        .with_secure(cookie_secure)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
        .with_name("dreamscroll_session");

    let static_asset_version = std::env::var("K_REVISION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let state = Arc::new(WebState {
        user_api,
        tera,
        static_asset_version,
    });

    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    let routes_open = Router::new()
        .route("/login", get(r_login_page::get).post(r_auth::login_post))
        .layer(auth_layer.clone());

    let routes_protected = Router::new()
        .route("/", get(r_index::get))
        .route("/cards", get(r_cards::get))
        .route("/command", post(r_command::post))
        .route("/upload", post(r_upload::post))
        .route("/logout", post(r_auth::logout_post))
        .layer(login_required!(auth::WebAuthBackend, login_url = "/login"))
        .layer(auth_layer);

    let mut router = Router::new()
        .merge(routes_protected)
        .merge(routes_open)
        .with_state(state);

    router = router.nest_service("/static", ServeDir::new("web/v2/static"));
    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router);
    router
}
