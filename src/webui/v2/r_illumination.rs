use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, serde::Deserialize)]
pub struct IlluminationParams {
    animate_illumination: Option<bool>,
}

async fn make_context(
    state: &WebState,
    context_user: &auth::Context,
    capture_id: i32,
    animate_illumination: bool,
) -> Result<Context, api::ApiError> {
    let page_snapshot_at = state.current_db_timestamp().await?;
    let capture = state.user_api.get_capture(context_user, capture_id).await?;

    let mut context = state.template_context();
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("capture", &capture);
    context.insert("animate_illumination", &animate_illumination);
    Ok(context)
}

/// Feed-card illumination fragment: summary, details, entities, and suggested
/// searches in one swappable fragment.
pub async fn feed_fragment(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    Query(params): Query<IlluminationParams>,
) -> Result<Response, api::ApiError> {
    let user = auth
        .user
        .expect("protected route requires an authenticated user");
    let context_user = user.into();
    let context = make_context(
        &state,
        &context_user,
        capture_id,
        params.animate_illumination.unwrap_or(false),
    )
    .await?;

    let rendered = state.render("partials/cards/capture_illumination.html.tera", &context)?;
    Ok(Html(rendered).into_response())
}

/// Detail-page illumination fragments: the summary/details fragment plus an
/// OOB replacement for the separate metadata article.
pub async fn detail_fragments(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    Query(params): Query<IlluminationParams>,
) -> Result<Response, api::ApiError> {
    let user = auth
        .user
        .expect("protected route requires an authenticated user");
    let context_user = user.into();
    let mut context = make_context(
        &state,
        &context_user,
        capture_id,
        params.animate_illumination.unwrap_or(false),
    )
    .await?;

    let illumination = state.render("partials/cards/detail_illumination.html.tera", &context)?;

    context.insert("oob", &true);
    let metadata = state.render("partials/cards/capture_metadata.html.tera", &context)?;

    Ok(Html(format!("{illumination}{metadata}")).into_response())
}
