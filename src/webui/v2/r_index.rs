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
    card::{FeedContent, blend_capture_and_spark_cards, cards_from_captures, load_spark_cards},
};

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    pub q: String,
    pub n: Option<u64>,
    pub mode: Option<String>,
}

impl FeedContent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Blend => "blend",
            Self::Captures => "captures",
            Self::Sparks => "sparks",
        }
    }
}

pub(super) fn resolve_mode(mode: Option<&str>) -> FeedContent {
    match mode {
        Some("captures") => FeedContent::Captures,
        Some("sparks") => FeedContent::Sparks,
        Some("blend") => FeedContent::Blend,
        _ => FeedContent::Blend,
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
    let is_search_mode = !q.is_empty() && !q.starts_with('/');
    let cards = if is_search_mode {
        let capture_infos = state.user_api.search(&context_user, q).await?;
        cards_from_captures(capture_infos)
    } else {
        match mode {
            FeedContent::Sparks => load_spark_cards(&state.user_api, &context_user, limit).await?,
            FeedContent::Captures => {
                let capture_infos = state
                    .user_api
                    .get_timeline(&context_user, Some(limit))
                    .await?;
                cards_from_captures(capture_infos)
            }
            FeedContent::Blend => {
                let capture_infos = state
                    .user_api
                    .get_timeline(&context_user, Some(limit))
                    .await?;
                let capture_cards = cards_from_captures(capture_infos);
                let spark_cards = load_spark_cards(&state.user_api, &context_user, limit).await?;
                blend_capture_and_spark_cards(capture_cards, spark_cards)
            }
        }
    };

    let mut context = state.template_context();
    context.insert("cards", &cards);
    context.insert("query", q);
    context.insert("feed_mode", mode.as_str());
    context.insert("is_search_mode", &is_search_mode);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
