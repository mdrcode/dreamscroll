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
    card::{blend_capture_and_spark_cards, cards_from_captures, cards_from_sparks},
};

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    pub q: String,
    pub n: Option<u64>,
    pub include_sparks: Option<bool>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<IndexQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();
    let limit = query.n.unwrap_or(30);

    let q = query.q.trim();
    let capture_infos = if q.is_empty() {
        state
            .user_api
            .get_timeline(&context_user, Some(limit))
            .await?
    } else {
        state.user_api.search(&context_user, q).await?
    };

    let capture_cards = cards_from_captures(capture_infos);
    let cards = if query.include_sparks.unwrap_or(false) {
        let spark_cards =
            cards_from_sparks(state.user_api.get_sparks(&context_user, None).await?, 3);
        blend_capture_and_spark_cards(capture_cards, spark_cards)
    } else {
        capture_cards
    };

    let mut context = state.template_context();
    context.insert("cards", &cards);
    context.insert("query", q);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
