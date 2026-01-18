use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use tera::Context;

use crate::{api, common::AppError};

use super::WebState;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    q: String,
}

#[tracing::instrument(skip(state, params))]
pub async fn search(
    State(state): State<Arc<WebState>>,
    Query(params): Query<SearchParams>,
) -> Result<Response, AppError> {
    let query = params.q.trim();
    let capture_infos = api::CaptureInfo::search_by_illuminations(&state.db, query).await?;

    let mut context = Context::new();
    context.insert("query", query);
    context.insert("capture_infos", &capture_infos);
    context.insert("result_count", &capture_infos.len());

    let rendered = state
        .tera
        .render("search.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
