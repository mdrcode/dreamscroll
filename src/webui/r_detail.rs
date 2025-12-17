use crate::common::collect_images;
use crate::webui::WebState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use std::sync::Arc;
use tera::Context;

pub async fn detail(State(state): State<Arc<WebState>>, Path(filename): Path<String>) -> Response {
    // Find the image with the matching filename
    let images = collect_images();
    let image = match images.into_iter().find(|img| img.filename == filename) {
        Some(img) => img,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let mut context = Context::new();
    context.insert("image", &image);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered).into_response()
}
