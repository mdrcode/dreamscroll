use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{OriginalUri, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::{
    WebState,
    card::{blend_capture_and_spark_cards, cards_from_captures, load_spark_cards},
};

#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    #[serde(default)]
    pub q: String,
    pub n: Option<u64>,
    pub include_sparks: Option<bool>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    original_uri: OriginalUri,
    headers: HeaderMap,
    Query(query): Query<FeedQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let q = query.q.trim();
    let limit = query.n.unwrap_or(30);

    let capture_infos = if q.starts_with("/") {
        crate::webui::slash_command::process(q, &context_user, &state.user_api).await?;
        state
            .user_api
            .get_timeline(&context_user, Some(limit))
            .await?
    } else if q.is_empty() {
        state
            .user_api
            .get_timeline(&context_user, Some(limit))
            .await?
    } else {
        state.user_api.search(&context_user, q).await?
    };

    let capture_cards = cards_from_captures(capture_infos);
    let cards = if query.include_sparks.unwrap_or(false) {
        let spark_cards = load_spark_cards(&state.user_api, &context_user, 3).await?;
        blend_capture_and_spark_cards(capture_cards, spark_cards)
    } else {
        capture_cards
    };

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    if !is_htmx {
        let canonical = match original_uri.0.query() {
            Some(query_string) if !query_string.is_empty() => format!("/v2?{}", query_string),
            _ => "/v2".to_string(),
        };
        return Ok(Redirect::to(&canonical).into_response());
    }

    let mut context = state.template_context();
    context.insert("cards", &cards);

    let rendered = state
        .tera
        .render("partials/feed.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
