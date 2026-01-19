use std::sync::Arc;

use axum::{Router, routing::get};

use crate::database::DbHandle;

pub mod r_capture;
pub mod r_dummy;
pub mod r_timeline;

pub struct ApiState {
    pub db: Arc<DbHandle>,
}

/// Creates the API router with all REST endpoints
pub fn make_api_router(db: Arc<DbHandle>) -> Router {
    let state = Arc::new(ApiState { db });

    Router::new()
        .route("/capture/{id}", get(r_capture::get))
        .route("/dummy", get(r_dummy::get))
        .route("/timeline", get(r_timeline::get))
        .with_state(state)
}
