use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::{Context, Tera};
use tower_http::services::ServeDir;
use tower_sessions::SessionManagerLayer;

use crate::{api, auth, facility};

use super::*;

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
    auth_backend: auth::WebAuthBackend,
    session_layer: SessionManagerLayer<impl tower_sessions::SessionStore + Clone>,
) -> Router {
    // Note, this will hang forever if templates fail to load (waiting on iterator)
    let tera = Tera::new("web/v1/templates/*.tera").expect("Failed to load templates");
    tracing::info!("Loaded tera templates");

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

    let routes_protected = Router::new()
        .route("/", get(r_index::get))
        .route("/sparks", get(r_sparks::get))
        .route("/search", get(r_search::get))
        .route("/detail/{capture_id}", get(r_detail::get))
        .route("/entity/knode/{id}", get(r_entity::get_knode))
        .route("/entity/social/{id}", get(r_entity::get_social_media))
        .route("/upload", post(r_upload::post))
        .layer(login_required!(
            auth::WebAuthBackend,
            login_url = "/v2/login"
        )) // MUST come first
        .layer(auth_layer);

    let mut router = axum::Router::new()
        .merge(routes_protected)
        .with_state(state);

    // For local dev, we serve static JS/CSS files directly
    router = router.nest_service("/static", ServeDir::new("web/v1/static"));

    router = router.layer(DefaultBodyLimit::max(5 * 1024 * 1024));
    router = facility::add_trace_propagation(router); // Cloud Run trace headers
    router
}
