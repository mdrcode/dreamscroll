use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use tera::Context;

use crate::{api, common::AppError};

use super::WebState;

pub async fn detail(
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, AppError> {
    let capture_info = api::fetch_capture_by_id(&state.db, id).await?;

    let mut context = Context::new();
    context.insert("capture", &capture_info);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
