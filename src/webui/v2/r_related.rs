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

    let captures = state
        .user_api
        .search_similar(&context_user, id, Some(12))
        .await?;

    let mut context = state.template_context();
    context.insert("captures", &captures);

    let rendered = state
        .tera
        .render("partials/cards/related_captures.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
