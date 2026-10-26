use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let page_snapshot_at = state.current_db_timestamp().await?;
    let capture = state.user_api.get_capture(&context_user, id).await?;

    let mut context = state.template_context();
    context.insert("page_kind", &"capture_detail");
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("feed_content_mode", "detail");
    context.insert("capture", &capture);
    context.insert("show_archive_actions", &true);
    // Archiving or unarchiving here leaves the user on the same page, with the
    // action flipped.
    context.insert("archive_return_to", &format!("/detail/{id}"));

    let rendered = state.render("detail.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}
