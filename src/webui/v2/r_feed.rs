use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, HeaderValue},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;
use tera::Context;

use crate::{api, auth};

use super::{
    WebState,
    content::{ContentSpec, render_content},
};

fn is_htmx_request(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == "true")
        .unwrap_or(false)
}

fn canonical_url(original_uri: &OriginalUri) -> String {
    match original_uri.0.query() {
        Some(query) if !query.is_empty() => format!("/?{query}"),
        _ => "/".to_string(),
    }
}

async fn make_context(
    state: &WebState,
    context_user: &auth::Context,
    query: &ContentSpec,
) -> Result<Context, api::ApiError> {
    let page_snapshot_at = state.current_db_timestamp().await?;
    let cards = render_content(&state.user_api, context_user, query).await?;

    let mut context = state.template_context();
    context.insert("page_kind", &"feed");
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("cards", &cards);
    Ok(context)
}

/// Full HTML feed page at `/`.
pub async fn page(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<ContentSpec>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let is_search = query.is_search();
    let q = query.search_query();
    let mut context = make_context(&state, &context_user, &query).await?;
    let feed_content_mode = if is_search { "search" } else { "timeline" };
    context.insert("feed_content_mode", &feed_content_mode);
    context.insert("query", q);

    let rendered = state.render("index.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}

/// HTMX feed fragment for `#card-feed`; non-HTMX requests redirect to the
/// canonical feed URL.
pub async fn fragment(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    Query(query): Query<ContentSpec>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let canonical = canonical_url(&original_uri);
    if !is_htmx_request(&headers) {
        return Ok(Redirect::to(&canonical).into_response());
    }

    let context = make_context(&state, &context_user, &query).await?;
    let rendered = state.render("partials/feed.html.tera", &context)?;

    let mut response = Html(rendered).into_response();
    let canonical_header = HeaderValue::from_str(&canonical)
        .map_err(|e| anyhow!("Failed to set HX-Push-Url header: {e}"))?;
    response
        .headers_mut()
        .insert("HX-Push-Url", canonical_header);

    Ok(response)
}
