use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, HeaderValue},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::{
    WebState,
    content::{ContentSpec, render_content},
};

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    Query(query): Query<ContentSpec>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let canonical = match original_uri.0.query() {
        Some(query_string) if !query_string.is_empty() => format!("/?{}", query_string),
        _ => "/".to_string(),
    };

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if !is_htmx {
        return Ok(Redirect::to(&canonical).into_response());
    }

    let cards = render_content(&state.user_api, &context_user, &query).await?;

    let mut context = state.template_context();
    context.insert("cards", &cards);

    let rendered = state
        .tera
        .render("partials/feed.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    let mut response = Html(rendered).into_response();
    let canonical_header = HeaderValue::from_str(&canonical)
        .map_err(|e| anyhow!("Failed to set HX-Push-Url header: {e}"))?;
    response
        .headers_mut()
        .insert("HX-Push-Url", canonical_header);

    Ok(response)
}
