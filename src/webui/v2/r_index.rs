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
    feed::{FeedContent, search_cards, timeline_cards},
};

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    pub q: String,
    pub n: Option<u64>,
    pub content: Option<FeedContent>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<IndexQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let content = query.content.unwrap_or(FeedContent::Blend);
    let limit = query.n.unwrap_or(50);

    let q = query.q.trim();
    let is_search_mode = !q.is_empty();
    let cards = if is_search_mode {
        search_cards(&state.user_api, &context_user, q).await?
    } else {
        timeline_cards(&state.user_api, &context_user, content, limit).await?
    };

    let mut context = state.template_context();
    context.insert("is_search_mode", &is_search_mode);
    context.insert("query", q);
    context.insert("cards", &cards);
    context.insert("content_mode", content.as_str());

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
