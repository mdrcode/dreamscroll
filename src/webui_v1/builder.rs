use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::{Router, routing::get, routing::post};
use tera::Tera;

use crate::database::DbHandle;
use crate::storage::StorageProvider;
use crate::webui_v1::r_detail::detail;
use crate::webui_v1::r_index::index;
use crate::webui_v1::r_search::search;
use crate::webui_v1::r_upload::upload;

pub struct WebState {
    pub db: Arc<DbHandle>,
    pub storage: Arc<dyn StorageProvider + Send + Sync>,
    pub tera: Tera,
}

pub fn make_axum_router(
    db: Arc<DbHandle>,
    storage: Arc<dyn StorageProvider + Send + Sync>,
) -> Router {
    let tera = Tera::new("web_templates/v1/*.tera").expect("Failed to load templates");
    let state = Arc::new(WebState { db, storage, tera });

    let router = Router::new()
        .route("/", get(index))
        .route("/search", get(search))
        .route("/detail/{capture_id}", get(detail))
        .route("/upload", post(upload))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024)) // 5 MB
        .with_state(state);

    router
}
