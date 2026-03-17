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
    card::{blend_capture_and_spark_cards, cards_from_captures, load_spark_cards},
};

#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    #[serde(default)]
    pub q: String,
    pub n: Option<u64>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedMode {
    Blend,
    Captures,
    Sparks,
}

fn resolve_mode(mode: Option<&str>) -> FeedMode {
    match mode {
        Some("captures") => FeedMode::Captures,
        Some("sparks") => FeedMode::Sparks,
        Some("blend") => FeedMode::Blend,
        _ => FeedMode::Blend,
    }
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

    let mode = resolve_mode(query.mode.as_deref());
    let did_process_slash_command = q.starts_with('/');

    let cards = match mode {
        FeedMode::Sparks => load_spark_cards(&state.user_api, &context_user, 3).await?,
        FeedMode::Captures => {
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
            cards_from_captures(capture_infos)
        }
        FeedMode::Blend => {
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
            let spark_cards = load_spark_cards(&state.user_api, &context_user, 3).await?;
            blend_capture_and_spark_cards(capture_cards, spark_cards)
        }
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

    let mut response = Html(rendered).into_response();
    if did_process_slash_command {
        response.headers_mut().insert(
            "HX-Trigger",
            HeaderValue::from_static("ds-clear-search-input"),
        );
    }

    Ok(response)
}
