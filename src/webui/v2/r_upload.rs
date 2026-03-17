use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    body,
    extract::State,
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::Multipart;
use axum_login::AuthSession;

use crate::{api, auth};

use super::{WebState, card::cards_from_captures};

pub async fn post(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let media_bytes = match extract_bytes(multipart, "image").await? {
        Some(bytes) => bytes,
        None => {
            return Err(api::ApiError::bad_request(anyhow!("No image data found.")));
        }
    };

    if media_bytes.len() > 5 * 1024 * 1024 {
        return Err(api::ApiError::payload_too_large(anyhow!(
            "Payload too large."
        )));
    }

    let cap = state
        .user_api
        .insert_capture(&context_user, media_bytes)
        .await?;
    tracing::info!("Capture {} inserted via v2 upload", cap.id);

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

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

async fn extract_bytes(mut mp: Multipart, field: &str) -> anyhow::Result<Option<body::Bytes>> {
    while let Ok(Some(f)) = mp.next_field().await {
        if f.name().unwrap_or("") != field {
            continue;
        }

        match f.bytes().await {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) => return Err(e.into()),
        };
    }

    Ok(None)
}
