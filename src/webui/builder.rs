use crate::db::DbHandle;
use crate::webui::r_detail::detail;
use crate::webui::r_index::index;
use crate::webui::r_upload::upload;
use axum::{Router, routing::get, routing::post};
use std::{fs, sync::Arc};
use tera::Tera;
use tower_http::services::ServeDir;

pub struct WebAppState {
    pub db: Arc<DbHandle>,
    pub tera: Tera,
}

pub fn build_axum_router(db: Arc<DbHandle>) -> Router {
    fs::create_dir_all("localdev/uploads").unwrap();

    let tera = Tera::new("templates/*.tera").expect("Failed to load templates");
    let state = Arc::new(WebAppState { db, tera });

    Router::new()
        .route("/", get(index))
        .route("/detail/{filename}", get(detail))
        .route("/upload", post(upload))
        .nest_service("/uploads", ServeDir::new("localdev/uploads"))
        .with_state(state)
}
