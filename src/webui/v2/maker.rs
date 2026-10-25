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
    // Enforce autoescaping for all HTML templates, to prevent XSS
    tera.autoescape_on(vec![".html.tera"]);
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

    let status_catchup = task_master.status_catchup();
    let state = Arc::new(WebState {
        user_api,
        task_master,
        status_catchup,
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
        .route("/", get(r_feed::page))
        .route("/detail/{id}", get(r_detail::get))
        .route("/entity/{id}", get(r_entity::get))
        .route(
            "/detail/{id}/illumination",
            get(r_illumination::detail_fragments),
        )
        .route("/detail/{id}/related/previews", get(r_related::previews))
        .route("/feed", get(r_feed::fragment))
        .route(
            "/feed/capture/{id}/illumination",
            get(r_illumination::feed_fragment),
        )
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
    fn illumination_fragments_render_without_capture_chrome() {
        let tera = load_templates().expect("all v2 templates should load");
        let mut context = tera::Context::new();
        context.insert("page_snapshot_at", &"2026-10-22T12:00:00Z");
        context.insert(
            "capture",
            &serde_json::json!({
                "id": 7,
                "illuminations": [{
                    "summary": "Summary text",
                    "details": "Details text",
                    "knowledge_entities": [{
                        "id": 11,
                        "type": "real_person",
                        "name": "Ada Lovelace",
                        "description": "Mathematician",
                        "search_query": "Ada Lovelace",
                        "platform_link": null
                    }],
                    "suggested_searches": ["Ada Lovelace"]
                }]
            }),
        );

        let feed = tera
            .render("partials/cards/capture_illumination.html.tera", &context)
            .expect("feed illumination should render");
        assert!(feed.contains("id=\"capture-illumination-7\""));
        assert!(feed.contains("data-has-illumination=\"true\""));
        assert!(feed.contains("Ada Lovelace"));
        assert!(feed.contains("href=\"/entity/11\""));
        assert!(feed.contains("aria-controls=\"capture-details-7\""));
        assert!(
            feed.contains("id=\"capture-details-7\" class=\"capture-card__details-panel\" inert")
        );
        assert!(!feed.contains("annotation-block"));

        let detail = tera
            .render("partials/cards/detail_illumination.html.tera", &context)
            .expect("detail illumination should render");
        assert!(detail.contains("Summary text"));
        assert!(detail.contains("aria-controls=\"capture-detail-summary-7\""));
        assert!(!detail.contains("Ada Lovelace"));

        context.insert("oob", &true);
        let metadata = tera
            .render("partials/cards/capture_metadata.html.tera", &context)
            .expect("detail metadata should render");
        assert!(metadata.contains("id=\"capture-metadata-7\""));
        assert!(metadata.contains("hx-swap-oob=\"outerHTML\""));
        assert!(metadata.contains("id=\"capture-metadata-content-7\""));
        assert!(metadata.contains("Ada Lovelace"));
        assert!(metadata.contains("href=\"/entity/11\""));

        let mut empty_context = tera::Context::new();
        empty_context.insert("page_snapshot_at", &"2026-10-22T12:00:00Z");
        empty_context.insert(
            "capture",
            &serde_json::json!({ "id": 8, "illuminations": [] }),
        );
        let empty_feed = tera
            .render(
                "partials/cards/capture_illumination.html.tera",
                &empty_context,
            )
            .expect("empty feed illumination should render");
        assert!(empty_feed.contains("data-has-illumination=\"false\""));
        let empty_detail = tera
            .render(
                "partials/cards/detail_illumination.html.tera",
                &empty_context,
            )
            .expect("empty detail illumination should render");
        assert!(empty_detail.contains("data-has-illumination=\"false\""));
    }

    #[test]
    fn page_layouts_use_explicit_page_kind() {
        let tera = load_templates().expect("all v2 templates should load");

        let mut feed = tera::Context::new();
        feed.insert("static_asset_version", &"test");
        feed.insert("page_kind", &"feed");
        feed.insert("feed_content_mode", &"timeline");
        feed.insert("query", &"");
        feed.insert("cards", &serde_json::json!([]));
        let feed = tera.render("index.html.tera", &feed).unwrap();
        assert!(feed.contains("data-page-kind=\"feed\""));
        assert!(feed.contains("hx-get=\"/feed\""));
        assert!(feed.contains("id=\"file-upload-form\""));
        assert!(feed.contains("aria-label=\"Search captures\""));

        let mut detail = tera::Context::new();
        detail.insert("static_asset_version", &"test");
        detail.insert("page_kind", &"capture_detail");
        detail.insert("feed_content_mode", &"detail");
        detail.insert("page_snapshot_at", &"2026-10-22T12:00:00Z");
        detail.insert(
            "capture",
            &serde_json::json!({
                "id": 7,
                "medias": [{ "url": "https://example.com/image.jpg", "storage_uuid": "abc" }],
                "illuminations": [],
                "annotation": null,
                "created_at": "2026-10-22T12:00:00Z",
                "created_at_human": "now"
            }),
        );
        let detail = tera.render("detail.html.tera", &detail).unwrap();
        assert!(detail.contains("data-page-kind=\"capture_detail\""));
        assert!(detail.contains("data-capture-id=\"7\""));
        assert!(detail.contains("action=\"/\""));
        assert!(!detail.contains("hx-get=\"/feed\""));

        let mut account = tera::Context::new();
        account.insert("static_asset_version", &"test");
        account.insert("page_kind", &"account");
        account.insert("csrf_token", &"token");
        account.insert("already_logged_in", &false);
        let account = tera.render("login.html.tera", &account).unwrap();
        assert!(account.contains("data-page-kind=\"account\""));
        assert!(!account.contains("header-search-form"));
        assert!(!account.contains("file-upload-form"));
        assert!(!account.contains("webui-v2.js"));
    }

    #[test]
    fn entity_page_renders_occurrence_and_source_capture() {
        let tera = load_templates().expect("all v2 templates should load");
        let mut context = tera::Context::new();
        context.insert("static_asset_version", &"test");
        context.insert("page_kind", &"entity_detail");
        context.insert(
            "entity",
            &serde_json::json!({
                "id": 11,
                "capture_id": 7,
                "name": "Ada Lovelace",
                "type": "real_person",
                "description": "Pioneer of computing",
                "search_query": "Ada Lovelace mathematician",
                "platform_link": {
                    "platform": "x_twitter",
                    "handle": "@ada",
                    "display_name": "Ada"
                }
            }),
        );
        context.insert(
            "capture",
            &serde_json::json!({
                "id": 7,
                "medias": [{ "url": "https://example.com/image.jpg" }],
                "illuminations": [{ "summary": "Source capture summary" }],
                "created_at_human": "just now"
            }),
        );
        context.insert(
            "siblings",
            &serde_json::json!([{
                "id": 12,
                "name": "Charles Babbage",
                "type": "real_person",
                "description": "Mathematician"
            }]),
        );

        let rendered = tera.render("entity.html.tera", &context).unwrap();

        assert!(rendered.contains("data-page-kind=\"entity_detail\""));
        assert!(rendered.contains("Entity occurrence"));
        assert!(rendered.contains("Ada Lovelace"));
        assert!(rendered.contains("Pioneer of computing"));
        assert!(
            rendered.contains("https://www.google.com/search?q=Ada%20Lovelace%20mathematician")
        );
        assert!(rendered.contains("href=\"/detail/7\""));
        assert!(rendered.contains("Source capture summary"));
        assert!(rendered.contains("href=\"/entity/12\""));
        assert!(rendered.contains("Charles Babbage"));
        assert!(!rendered.contains("annotation-block"));
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
                            "thumbnail_url": "https://example.com/image.jpg",
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
