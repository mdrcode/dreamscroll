use crate::controller::collect_images_db;
use crate::webui::WebState;
use axum::{extract::State, response::Html};
use std::sync::Arc;
use tera::Context;

pub async fn index(State(state): State<Arc<WebState>>) -> Html<String> {
    // Collect all uploaded images with timestamps
    let mut images = collect_images_db(&state.db).await;

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let mut context = Context::new();
    context.insert("images", &images);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered)
}
