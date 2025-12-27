use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use sea_orm::entity::prelude::*;
use tera::Context;

use crate::common::CaptureInfo;
use crate::model::{capture, media};
use crate::webui::WebState;

pub async fn detail(State(state): State<Arc<WebState>>, Path(capture_id): Path<i32>) -> Response {
    let captures = capture::Entity::find_by_id(capture_id)
        .find_with_related(media::Entity)
        .all(&state.db.conn)
        .await
        .expect(&format!("Failed to fetch capture {} from db.", capture_id));

    let capture_info = if let Some((capture, medias)) = captures.into_iter().next() {
        CaptureInfo { capture, medias }
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut context = Context::new();
    context.insert("cm", &capture_info);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .expect("Failed to render template");

    Html(rendered).into_response()
}
