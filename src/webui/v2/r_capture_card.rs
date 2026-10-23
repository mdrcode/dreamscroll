use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, serde::Deserialize)]
pub struct CardParams {
    animate_illumination: Option<bool>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
    Query(params): Query<CardParams>,
) -> Result<Response, api::ApiError> {
    let user = auth
        .user
        .expect("protected route requires an authenticated user");
    let context_user = user.into();
    let page_snapshot_at = state.current_db_timestamp().await?;
    let capture = state.user_api.get_capture(&context_user, id).await?;

    let card = super::content::Card::Capture(super::content::CaptureCard { capture });
    let mut context = state.template_context();
    context.insert("page_snapshot_at", &page_snapshot_at);
    context.insert("card", &card);
    context.insert(
        "animate_illumination",
        &params.animate_illumination.unwrap_or(false),
    );
    let rendered = state
        .tera
        .render("partials/card.html.tera", &context)
        .map_err(|error| anyhow!("Failed to render capture card: {error}"))?;

    Ok(Html(rendered).into_response())
}
