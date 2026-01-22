use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[derive(Deserialize)]
pub struct LoginParams {
    error: Option<String>,
}

pub async fn login_page(
    auth: AuthSession<auth::WebAuthBackend>,
    Query(params): Query<LoginParams>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let mut context = Context::new();

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
