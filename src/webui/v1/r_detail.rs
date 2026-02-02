use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[tracing::instrument(skip(auth, state, id))]
pub async fn detail(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    tracing::debug!("Rendering detail for capture {} for user {}", id, user.id());

    let fetch = state
        .api_client
        .get_captures(&user.into(), Some(vec![id]))
        .await?;

    let capture = fetch
        .into_iter()
        .next()
        .ok_or_else(|| api::ApiError::not_found(anyhow!("Capture with id {} not found", id)))?;

    let mut context = Context::new();
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
