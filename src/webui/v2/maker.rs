use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::{Context, Tera};
use tower_http::services::{ServeDir, ServeFile};
use tower_sessions::SessionManagerLayer;

use crate::{api, auth, sse, task, telemetry};

use super::*;

pub struct WebState {
    pub user_api: api::UserApiClient,
    pub task_master: Arc<task::TaskMaster>,
    pub server_events: tokio::sync::broadcast::Sender<sse::ReceivedServerEvent>,
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    pub tera: Tera,
    pub static_asset_version: String,
    pub max_upload_bytes: usize,
}

impl WebState {
    pub fn template_context(&self) -> Context {
        let mut context = Context::new();
        context.insert("static_asset_version", &self.static_asset_version);
        context
    }
}

fn load_templates() -> Result<Tera, tera::Error> {
    let mut tera = Tera::new();
    tera.register_filter("json_encode", tera_contrib::json::json_encode);
    tera.register_filter("urlencode", tera_contrib::urlencode::urlencode);
    tera.load_from_glob("web/v2/templates/**/*.tera")?;
    Ok(tera)
}

pub fn make_ui_router(
    user_api: api::UserApiClient,
    task_master: Arc<task::TaskMaster>,
    server_events: tokio::sync::broadcast::Sender<sse::ReceivedServerEvent>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    auth_backend: auth::WebAuthBackend,
    session_layer: SessionManagerLayer<impl tower_sessions::SessionStore + Clone>,
    max_upload_bytes: usize,
) -> Router {
    let tera = load_templates().expect("Failed to load v2 templates");
    tracing::info!("Loaded v2 tera templates");

    let static_asset_version = std::env::var("K_REVISION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let mut hasher = blake3::Hasher::new();
            for path in [
                "web/v2/static/dreamscroll-v2.css",
                "web/v2/static/webui-v2.js",
            ] {
                if let Ok(contents) = std::fs::read(path) {
                    hasher.update(&contents);
                }
            }
            format!(
                "{}-{}",
                env!("CARGO_PKG_VERSION"),
                &hasher.finalize().to_hex()[..12]
            )
        });

    let state = Arc::new(WebState {
        user_api,
        task_master,
        server_events,
        shutdown,
        tera,
        static_asset_version,
        max_upload_bytes,
    });

    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    let routes_open = Router::new()
        .route("/login", get(r_login_page::get).post(r_auth::login_post))
        .route_service(
            "/manifest.webmanifest",
            ServeFile::new("web/v2/static/manifest.webmanifest"),
        )
        .route_service("/sw.js", ServeFile::new("web/v2/static/sw.js"))
        .layer(auth_layer.clone());

    let routes_protected = Router::new()
        .route("/", get(r_index::get))
        .route("/detail/{id}", get(r_detail::get))
        .route("/detail/{id}/partial", get(r_detail_partial::get))
        .route("/detail/{id}/related", get(r_related::get))
        .route("/cards", get(r_cards::get))
        .route("/cards/capture/{id}", get(r_capture_card::get))
        .route("/events", get(r_events::get))
        .route(
            "/annotation/{capture_id}",
            get(r_annotation::block).post(r_annotation::set),
        )
        .route("/annotation/{capture_id}/form", get(r_annotation::form))
        .route(
            "/annotation/{capture_id}/archive",
            post(r_annotation::archive),
        )
        .route("/masonry", get(r_masonry::get))
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
    router = router.layer(DefaultBodyLimit::max(max_upload_bytes));
    router = telemetry::add_axum_trace_propagation(router);
    router
}

#[cfg(test)]
mod tests {
    use super::load_templates;

    #[test]
    fn templates_load_successfully() {
        load_templates().expect("all v2 templates should load");
    }
}
