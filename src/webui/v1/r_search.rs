use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use serde::Deserialize;
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    q: String,
}

#[tracing::instrument(skip(auth, state, params))]
pub async fn search(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(params): Query<SearchParams>,
) -> Result<Response, api::AppError> {
    let query = params.q.trim();

    let user = auth.user.unwrap();
    tracing::debug!("Rendering search q: {} for user ID {}", query, user.id());

    let capture_infos: Vec<_> =
        api::search_by_illuminations(auth::Context::from(user), &state.db, query).await?;

    let mut context = Context::new();
    context.insert("query", query);
    context.insert("result_count", &capture_infos.len());
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("search.html.tera", &context)
        .map_err(|e| api::AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
