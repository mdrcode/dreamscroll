use std::sync::Arc;

use axum::{
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

pub async fn post(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let cap = crate::webui::upload::insert_capture_from_multipart(
        &state.user_api,
        &context_user,
        multipart,
    )
    .await?;
    tracing::info!("Capture {} inserted via v2 upload", cap.id);

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if !is_htmx {
        return Ok(Redirect::to("/").into_response());
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}
