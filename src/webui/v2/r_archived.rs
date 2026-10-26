use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::{
    WebState,
    content::{CaptureCard, Card},
};

/// Archived captures are a recovery surface, not a feed: one page, no paging.
const ARCHIVED_LIMIT: u64 = 100;

/// Full HTML page at `/archived`: the user's archived captures, most recently
/// archived first. Deliberately unlinked from navigation for now.
pub async fn page(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let page_snapshot_at = state.current_db_timestamp().await?;
    let captures = state
        .user_api
        .get_captures_archived(&context_user, ARCHIVED_LIMIT)
        .await?;
    let cards: Vec<Card> = captures
        .into_iter()
        .map(|capture| Card::Capture(CaptureCard { capture }))
        .collect();

    let mut context = state.template_context();
    context.insert("page_kind", &"feed");
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("feed_content_mode", &"archived");
    context.insert("page_title", &"dreamscroll - archived");
    context.insert("feed_heading", &"Archived");
    context.insert("feed_empty_message", &"No archived captures.");
    context.insert("cards", &cards);
    context.insert("show_archive_actions", &true);
    // The card action posts back to this page.
    context.insert("archive_return_to", &"/archived");

    let rendered = state.render("index.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}
