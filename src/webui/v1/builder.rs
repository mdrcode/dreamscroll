use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use axum_login::AuthManagerLayerBuilder;
use tera::Tera;
use tower_sessions::{MemoryStore, SessionManagerLayer};

use crate::{auth, database::DbHandle, storage::StorageProvider};

use super::*;

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

    let state = Arc::new(WebState {
        db: db.clone(),
        storage,
        tera,
    });

    // Todo the session store should come externally from the "environment" and be part
    // of Facility/ config, etc
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store).with_secure(false); // TODO: Use secure cookies in production
    let auth_backend = auth::Backend::new(db.clone());
    let auth_layer = AuthManagerLayerBuilder::new(auth_backend, session_layer).build();

    let router = Router::new()
        .route("/login", get(login_page).post(login_handler))
        .route("/logout", get(logout_handler))
        .route("/", get(index))
        .route("/search", get(search))
        .route("/detail/{capture_id}", get(detail))
        .route("/upload", post(upload))
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024)) // 5 MB
        .with_state(state);

    router
}
