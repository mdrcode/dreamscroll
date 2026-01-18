use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use tera::Context;

use crate::{api, common::AppError};

use super::WebState;

#[tracing::instrument(skip(state))]
pub async fn index(State(state): State<Arc<WebState>>) -> Result<Response, AppError> {
    let capture_infos = api::fetch_timeline(&state.db).await?;

    let mut context = Context::new();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
