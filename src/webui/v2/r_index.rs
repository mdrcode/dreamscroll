use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::{
    WebState,
    content::{ContentQuery, cards_for_query},
};

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<ContentQuery>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let is_search_mode = query.is_search_mode();
    let cards = cards_for_query(&state.user_api, &context_user, &query).await?;
    let q = query.query_text();
    let content = query.content_mode();

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
