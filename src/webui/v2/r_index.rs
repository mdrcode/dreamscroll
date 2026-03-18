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
    content::{ContentSpec, render_content},
};

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(query): Query<ContentSpec>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let is_search = query.is_search();
    let cards = render_content(&state.user_api, &context_user, &query).await?;
    let q = query.search_query();
    let content = query.content_mode();

    let mut context = state.template_context();
    context.insert("is_search", &is_search);
    context.insert("query", q);
    context.insert("cards", &cards);
    context.insert("content_mode", &content);

    let rendered = state
        .tera
        .render("index.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
