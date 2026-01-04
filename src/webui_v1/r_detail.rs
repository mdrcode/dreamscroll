use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use tera::Context;

use crate::common::AppError;
use crate::controller::CaptureInfo;
use crate::webui_v1::WebState;

pub async fn detail(
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let capture_info = CaptureInfo::fetch_by_id(&state.db, id).await?;

    let mut context = Context::new();
    context.insert("capture", &capture_info);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
