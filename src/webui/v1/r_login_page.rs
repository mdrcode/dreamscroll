use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;
use tera::Context;
use tower_sessions::Session;

use crate::{api, auth};

use super::WebState;

#[derive(Deserialize)]
pub struct LoginParams {
    error: Option<String>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    session: Session,
    Query(params): Query<LoginParams>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let mut context = Context::new();

    // CSRF token: generate a fresh one on every GET and store it in the session.
    let csrf_token = uuid::Uuid::new_v4().to_string();
    session
        .insert("login_csrf_token", &csrf_token)
        .await
        .map_err(|e| anyhow!("Session error storing CSRF token: {}", e))?;
    context.insert("csrf_token", &csrf_token);

    // Check if user is already logged in
    if let Some(user) = auth.user {
        context.insert("already_logged_in", &true);
        context.insert("username", &user.username());
    } else {
        context.insert("already_logged_in", &false);
        if let Some(error_msg) = params.error {
            context.insert("error", &error_msg);
        }
    }

    let rendered = state
        .tera
        .render("login.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
