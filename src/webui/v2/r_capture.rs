use std::sync::Arc;

use axum::{
    Form,
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

/// Full HTML page for one capture.
pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let page_snapshot_at = state.current_db_timestamp().await?;
    let capture = state
        .user_api
        .get_capture(&context_user, capture_id)
        .await?;

    let mut context = state.template_context();
    context.insert("page_kind", &"capture_detail");
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("feed_content_mode", "detail");
    context.insert("capture", &capture);
    context.insert("show_archive_actions", &true);
    // Archiving or unarchiving here leaves the user on the same page, with the
    // action flipped.
    context.insert("archive_return_to", &format!("/capture/{capture_id}"));

    let rendered = state.render("capture.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}

#[derive(Debug, Deserialize)]
pub struct ArchiveAction {
    /// Where to go after the action, so the user stays on the page they clicked
    /// from. Restricted to local paths below.
    #[serde(default)]
    next: String,
}

/// Only local paths are honored, so the form cannot be used as an open redirect.
///
/// Browsers normalize `\` to `/` for http(s), so `/\evil.example` would become the
/// protocol-relative `//evil.example` and leave the site. Control characters have
/// no business in a `Location` path either.
fn redirect_target(next: &str, fallback: &str) -> String {
    let is_local = next.starts_with('/')
        && !next.starts_with("//")
        && !next.contains('\\')
        && !next.chars().any(char::is_control);

    if is_local {
        next.to_string()
    } else {
        fallback.to_string()
    }
}

pub async fn archive(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    Form(action): Form<ArchiveAction>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    state
        .user_api
        .archive_capture(&context_user, capture_id)
        .await?;

    Ok(Redirect::to(&redirect_target(&action.next, "/")).into_response())
}

pub async fn unarchive(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    Form(action): Form<ArchiveAction>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    state
        .user_api
        .unarchive_capture(&context_user, capture_id)
        .await?;

    Ok(Redirect::to(&redirect_target(&action.next, "/archived")).into_response())
}

#[cfg(test)]
mod tests {
    use super::redirect_target;

    #[test]
    fn redirect_target_only_honors_local_paths() {
        assert_eq!(redirect_target("/archived", "/"), "/archived");
        assert_eq!(redirect_target("/capture/7", "/"), "/capture/7");
        assert_eq!(redirect_target("", "/"), "/");

        assert_eq!(redirect_target("archived", "/"), "/");
        assert_eq!(redirect_target("https://evil.example", "/"), "/");
        assert_eq!(redirect_target("//evil.example", "/"), "/");
        assert_eq!(redirect_target("/\\evil.example", "/"), "/");
        assert_eq!(redirect_target("/\tevil.example", "/"), "/");
    }
}
