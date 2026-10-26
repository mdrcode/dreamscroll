use std::sync::Arc;

use axum::{
    Form,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

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
        assert_eq!(redirect_target("/detail/7", "/"), "/detail/7");
        assert_eq!(redirect_target("", "/"), "/");

        assert_eq!(redirect_target("archived", "/"), "/");
        assert_eq!(redirect_target("https://evil.example", "/"), "/");
        assert_eq!(redirect_target("//evil.example", "/"), "/");
        assert_eq!(redirect_target("/\\evil.example", "/"), "/");
        assert_eq!(redirect_target("/\tevil.example", "/"), "/");
    }
}
