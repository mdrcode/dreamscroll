use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use tera::Context;

use crate::{api, auth, common::AppError};

use super::WebState;

#[tracing::instrument(skip(auth, state))]
pub async fn index(
    auth: AuthSession<auth::Backend>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, AppError> {
    let user = auth.user.unwrap();
    tracing::debug!("Rendering index for user ID {}", user.id());

    let capture_infos = api::fetch_timeline(auth::Context::from(&user), &state.db).await?;

    let mut context = Context::new();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| AppError::internal(anyhow!("Failed to render template: {:?}", e)))?;

    Ok(Html(rendered).into_response())
}
