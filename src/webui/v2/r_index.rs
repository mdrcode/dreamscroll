use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::{
    WebState,
    card::{blend_capture_and_spark_cards, cards_from_captures, load_spark_cards},
};

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
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

impl FeedMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blend => "blend",
            Self::Captures => "captures",
            Self::Sparks => "sparks",
        }
    }
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
    Query(query): Query<IndexQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();
    let limit = query.n.unwrap_or(50);
    let mode = resolve_mode(query.mode.as_deref());

    let q = query.q.trim();
    let cards = match mode {
        FeedMode::Sparks => load_spark_cards(&state.user_api, &context_user, limit).await?,
        FeedMode::Captures => {
            let capture_infos = if q.is_empty() {
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
            let capture_infos = if q.is_empty() {
                state
                    .user_api
                    .get_timeline(&context_user, Some(limit))
                    .await?
            } else {
                state.user_api.search(&context_user, q).await?
            };
            let capture_cards = cards_from_captures(capture_infos);
            let spark_cards = load_spark_cards(&state.user_api, &context_user, limit).await?;
            blend_capture_and_spark_cards(capture_cards, spark_cards)
        }
    };

    let mut context = state.template_context();
    context.insert("cards", &cards);
    context.insert("query", q);
    context.insert("feed_mode", mode.as_str());

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
