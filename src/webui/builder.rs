use crate::webui::r_detail::detail;
use crate::webui::r_index::index;
use crate::webui::r_upload::upload;
use crate::{db::DbHandle, facility::Facility};
use axum::{Router, routing::get, routing::post};
use std::{fs, sync::Arc};
use tera::Tera;
use tower_http::services::ServeDir;

pub struct WebState {
    pub facility: Box<dyn Facility>,
    pub db: Arc<DbHandle>,
    pub tera: Tera,
}

pub fn build_axum_router(db: Arc<DbHandle>, facility: Box<dyn Facility>) -> Router {
    fs::create_dir_all(facility.local_media_path()).unwrap();

    let tera = Tera::new("templates/*.tera").expect("Failed to load templates");
    let state = Arc::new(WebState { facility, db, tera });

    Router::new()
        .route("/", get(index))
        .route("/detail/{filename}", get(detail))
        .route("/upload", post(upload))
        .nest_service("/uploads", ServeDir::new(state.facility.local_media_path()))
        .with_state(state)
}
