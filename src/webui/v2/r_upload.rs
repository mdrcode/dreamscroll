use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::AuthSession;

use crate::{api, auth};

use super::{WebState, content::cards_from_captures};

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

    let is_client_managed = headers
        .get("X-DS-Upload-Client")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if is_client_managed {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    if !is_htmx {
        return Ok(Redirect::to("/v2").into_response());
    }

    let captures = state.user_api.get_timeline(&context_user, Some(30)).await?;
    let cards = cards_from_captures(captures);

    let mut context = state.template_context();
    context.insert("cards", &cards);

    let rendered = state
        .tera
        .render("partials/feed.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
