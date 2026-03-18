use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::{AuthSession, AuthUser};
use serde::Deserialize;

use crate::{api, auth};

use super::*;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    q: String,
    n: Option<u64>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(params): Query<SearchParams>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let query = params.q.trim();
    tracing::debug!("Rendering search q: {} for user ID {}", query, user.id());

    if query.starts_with("/") {
        crate::webui::slash_command::process(query, &user.into(), &state.user_api).await?;
        return Ok(Redirect::to("/v1").into_response());
    }

    let capture_infos: Vec<_> = state.user_api.search(&user.into(), query, params.n).await?;

    let mut context = state.template_context();
    context.insert("query", query);
    context.insert("result_count", &capture_infos.len());
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("search.html.tera", &context)
        .map_err(|e| api::ApiError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
