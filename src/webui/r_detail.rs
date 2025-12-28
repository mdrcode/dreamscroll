use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use tera::Context;

use crate::controller::CaptureInfo;
use crate::webui::WebState;

pub async fn detail(State(state): State<Arc<WebState>>, Path(capture_id): Path<i32>) -> Response {
    let capture_info = CaptureInfo::fetch_by_id(&state.db, capture_id)
        .await
        .expect("Failed to fetch capture info");

    let mut context = Context::new();
    context.insert("capture", &capture_info);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered).into_response()
}
