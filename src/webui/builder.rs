use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::{Router, routing::get, routing::post};
use tera::Tera;
use tower_http::services::ServeDir;

use crate::database::DbHandle;
use crate::storage::StorageProvider;
use crate::webui::r_detail::detail;
use crate::webui::r_index::index;
use crate::webui::r_upload::upload;

pub struct WebState {
    pub db: Arc<DbHandle>,
    pub storage: Arc<dyn StorageProvider + Send + Sync>,
    pub tera: Tera,
}

pub fn build_axum_router(
    db: Arc<DbHandle>,
    storage: Arc<dyn StorageProvider + Send + Sync>,
) -> Router {
    let local_serving_path_opt = storage.local_serving_path();
    let tera = Tera::new("templates/*.tera").expect("Failed to load templates");
    let state = Arc::new(WebState { db, storage, tera });

    let mut router = Router::new()
        .route("/", get(index))
        .route("/detail/{capture_id}", get(detail))
        .route("/upload", post(upload))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024)) // 5 MB
        .with_state(state);

    if let Some(ref path) = local_serving_path_opt {
        // In some environments, we serve media directly from local storage
        router = router.nest_service("/media", ServeDir::new(path));
    }

    router
}
