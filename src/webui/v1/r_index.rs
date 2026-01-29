use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[tracing::instrument(skip(auth, state))]
pub async fn index(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    tracing::debug!("Rendering index for user ID {}", user.id());

    let capture_infos = api::fetch_timeline(&state.db, user.into()).await?;

    let mut context = Context::new();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
