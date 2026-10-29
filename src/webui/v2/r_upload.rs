use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    http::header,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::AuthSession;
use serde::Serialize;

use crate::{api, auth};

use super::WebState;

#[derive(Serialize)]
struct UploadResult {
    capture_id: i32,
    capture_url: String,
}

pub async fn post(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user: auth::Context = user.into();

    // Reject before the stream is polled: the body limit knows the cap, not the size.
    if let Some(body_bytes) = content_length(&headers)
        && body_bytes > state.max_upload_bytes as u64
    {
        return Err(api::ApiError::upload_too_large(
            state.max_upload_bytes,
            Some(body_bytes),
        ));
    }

    let cap = crate::webui::upload::insert_capture_from_multipart(
        &state.user_api,
        &context_user,
        multipart,
        state.max_upload_bytes,
    )
    .await?;

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if !is_htmx {
        return Ok(Redirect::to("/").into_response());
    }

    let payload = UploadResult {
        capture_id: cap.id,
        capture_url: format!("/capture/{}", cap.id),
    };

    Ok((StatusCode::CREATED, Json(payload)).into_response())
}

/// Declared body size, when the client sent one. Browsers always set it for
/// `FormData`; a chunked or absent header falls through to the body limit.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}
