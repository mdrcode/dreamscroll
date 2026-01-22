use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use tera::Context;

use crate::api;

use super::WebState;

pub async fn login_page(State(state): State<Arc<WebState>>) -> Result<Response, api::ApiError> {
    let context = Context::new();
    // TODO possibly pass in an error message?

    let rendered = state
        .tera
        .render("login.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
