use std::sync::Arc;

use axum::{extract::State, response::Html};
use tera::Context;

use crate::controller::CaptureInfo;
use crate::webui::WebState;

pub async fn index(State(state): State<Arc<WebState>>) -> Html<String> {
    let capture_infos = CaptureInfo::fetch_timeline(&state.db)
        .await
        .expect("Failed to fetch capture infos");

    let mut context = Context::new();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered)
}
