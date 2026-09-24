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
    let user = auth
        .user
        .expect("protected route requires an authenticated user");
    let context_user = user.into();
    let page_snapshot_at = state.current_db_timestamp().await?;
    let captures = state.user_api.get_captures(&context_user, vec![id]).await?;
    let capture = captures
        .into_iter()
        .next()
        .ok_or_else(|| api::ApiError::not_found(anyhow!("Capture with id {} not found", id)))?;

    let card = super::content::Card::Capture(super::content::CaptureCard { capture });
    let mut context = state.template_context();
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("card", &card);
    let rendered = state
        .tera
        .render("partials/card.html.tera", &context)
        .map_err(|error| anyhow!("Failed to render capture card: {error}"))?;

    Ok(Html(rendered).into_response())
}
