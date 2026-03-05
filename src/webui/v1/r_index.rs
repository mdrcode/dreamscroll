use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    n: Option<u64>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<IndexQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_id = user.id();

    let capture_infos = state.user_api.get_timeline(&user.into(), query.n).await?;
    tracing::info!(
        "Got {} capture infos for user ID {}",
        capture_infos.len(),
        user_id
    );

    let mut context = state.template_context();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
