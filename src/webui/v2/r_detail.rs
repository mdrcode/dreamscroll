use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let captures = state.user_api.get_captures(&context_user, vec![id]).await?;

    let capture = captures
        .into_iter()
        .next()
        .ok_or_else(|| api::ApiError::not_found(anyhow!("Capture with id {} not found", id)))?;

    let mut context = state.template_context();
    context.insert("feed_content_mode", "detail");
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("detail.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
