use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, HeaderValue},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::{
    WebState,
    content::{FeedContent, search_cards, timeline_cards},
};

#[derive(Debug, Deserialize)]
pub struct CardsContentQuery {
    #[serde(default)]
    pub q: String,
    pub n: u64,
    pub content: FeedContent,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    Query(query): Query<CardsContentQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let canonical = match original_uri.0.query() {
        Some(query_string) if !query_string.is_empty() => format!("/v2?{}", query_string),
        _ => "/v2".to_string(),
    };

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if !is_htmx {
        return Ok(Redirect::to(&canonical).into_response());
    }

    let q = query.q.trim();

    let cards = if q.is_empty() {
        timeline_cards(&state.user_api, &context_user, query.content, query.n).await?
    } else {
        search_cards(&state.user_api, &context_user, q).await?
    };

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
