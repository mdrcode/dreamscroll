use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Form, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct CommandForm {
    #[serde(default)]
    pub raw_command: String,
}

pub async fn post(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Form(form): Form<CommandForm>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let raw_command = form.raw_command.trim();

    if !raw_command.starts_with('/') {
        return Err(anyhow!("Expected slash command in raw_command parameter.").into());
    }

    crate::webui::slash_command::process(raw_command, &context_user, &state.user_api).await?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        "HX-Trigger",
        HeaderValue::from_static("ds-clear-search-input"),
    );

    Ok(response)
}
