use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, middleware, routing::get, routing::post};
use axum_login::{AuthManagerLayerBuilder, login_required};
use tera::Tera;
use tower_http::services::{ServeDir, ServeFile};
use tower_sessions::SessionManagerLayer;

use crate::{api, auth, sse, task, telemetry};

use super::static_cache::{
    manifest_cache_headers, service_worker_cache_headers, static_asset_cache_headers,
};
use super::*;

fn load_templates() -> Result<Tera, tera::Error> {
    let mut tera = Tera::new();
    // Autoescape is keyed to the full template suffix. Tera's defaults do not
    // match `.html.tera`, so without this every `{{ value }}` renders raw HTML.
    // Use the whole suffix so future non-HTML templates (e.g. `.txt.tera`) are
    // not accidentally HTML-escaped.
    tera.autoescape_on(vec![".html.tera"]);
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
            // MUST OMIT sw.js: its stable URL is no-cache so browsers
            // can discover service-worker updates.
            for path in [
                "web/v2/static/app-icon-192x192.png",
                "web/v2/static/app-icon-512x512.png",
                "web/v2/static/app-icon.svg",
                "web/v2/static/dreamscroll-v2.css",
                "web/v2/static/favicon-32x32.png",
                "web/v2/static/manifest.webmanifest",
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
            tower::ServiceBuilder::new()
                .layer(middleware::from_fn(manifest_cache_headers))
                .service(ServeFile::new("web/v2/static/manifest.webmanifest")),
        )
        .route_service(
            "/sw.js",
            tower::ServiceBuilder::new()
                .layer(middleware::from_fn(service_worker_cache_headers))
                .service(ServeFile::new("web/v2/static/sw.js")),
        )
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
        .route("/command", post(r_command::post))
        .route("/upload", post(r_upload::post))
        .route("/logout", post(r_auth::logout_post))
        .layer(login_required!(auth::WebAuthBackend, login_url = "/login"))
        .layer(auth_layer);

    let mut router = Router::new()
        .merge(routes_protected)
        .merge(routes_open)
        .with_state(state);

    router = router.nest_service(
        "/static",
        tower::ServiceBuilder::new()
            .layer(middleware::from_fn(static_asset_cache_headers))
            .service(ServeDir::new("web/v2/static")),
    );
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

    #[test]
    fn html_templates_escape_untrusted_content() {
        let tera = load_templates().expect("all v2 templates should load");
        let mut context = tera::Context::new();
        context.insert(
            "capture",
            &serde_json::json!({
                "id": 7,
                "annotation": { "content": "<script>alert(1)</script>" }
            }),
        );

        let rendered = tera
            .render("partials/cards/annotation_block.html.tera", &context)
            .expect("annotation block should render");

        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!rendered.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn spark_card_renders_clusters_without_wrapper() {
        let tera = load_templates().expect("all v2 templates should load");
        let mut context = tera::Context::new();
        context.insert(
            "card",
            &serde_json::json!({
                "spark": {
                    "created_at": "2026-10-22T12:00:00Z",
                    "created_at_human": "just now",
                    "meta": null,
                    "spark_clusters": [{
                        "title": "Test cluster",
                        "summary": "Cluster summary",
                        "spark_links": [{
                            "url": "https://example.com",
                            "commentary": "Link commentary"
                        }],
                        "capture_previews": [{
                            "id": 42,
                            "url": "https://example.com/image.jpg",
                            "summary": "Capture summary"
                        }]
                    }]
                }
            }),
        );

        let rendered = tera
            .render("partials/cards/spark.html.tera", &context)
            .expect("spark card should render");

        assert!(rendered.contains("Test cluster"));
        assert!(rendered.contains("https://example.com/image.jpg"));
    }
}
