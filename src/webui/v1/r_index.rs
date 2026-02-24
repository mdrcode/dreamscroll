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

pub async fn index(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let user_id = user.id();

    let capture_infos = state.user_api.get_timeline(&user.into()).await?;
    tracing::info!(
        "Got {} capture infos for user ID {}",
        capture_infos.len(),
        user_id
    );

    let mut context = Context::new();
    context.insert("capture_infos", &capture_infos);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
