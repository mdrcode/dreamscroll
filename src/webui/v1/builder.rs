use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use tera::Tera;

use crate::{database::DbHandle, storage::StorageProvider};

use super::{r_detail::detail, r_index::index, r_search::search, r_upload::upload};

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
