use crate::common::collect_images;
use crate::webui::WebAppState;
use axum::{extract::State, response::Html};
use std::sync::Arc;
use tera::Context;

pub async fn index(State(state): State<Arc<WebAppState>>) -> Html<String> {
    // Collect all uploaded images with timestamps
    let mut images = collect_images();

    // Sort by timestamp descending (most recent first)
    images.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let mut context = Context::new();
    context.insert("images", &images);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered)
}
