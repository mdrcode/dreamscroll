use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Form, Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;
use serde::Deserialize;

use crate::{api, auth};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct AnnotationForm {
    pub content: String,
}

pub async fn form(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let captures = state.user_api.get_captures(&context_user, vec![capture_id]).await?;
    let capture = captures.into_iter().next().ok_or_else(|| {
        api::ApiError::not_found(anyhow!("Capture with id {} not found", capture_id))
    })?;

    let mut context = state.template_context();
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("partials/cards/annotation_editor.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}

pub async fn block(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let captures = state.user_api.get_captures(&context_user, vec![capture_id]).await?;
    let capture = captures.into_iter().next().ok_or_else(|| {
        api::ApiError::not_found(anyhow!("Capture with id {} not found", capture_id))
    })?;

    let mut context = state.template_context();
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("partials/cards/annotation_block.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}

pub async fn set(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    Form(form): Form<AnnotationForm>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    state
        .user_api
        .set_annotation(&context_user, capture_id, form.content)
        .await?;

    let captures = state.user_api.get_captures(&context_user, vec![capture_id]).await?;
    let capture = captures.into_iter().next().ok_or_else(|| {
        api::ApiError::not_found(anyhow!("Capture with id {} not found", capture_id))
    })?;

    let mut context = state.template_context();
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("partials/cards/annotation_block.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}

pub async fn archive(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    state
        .user_api
        .archive_annotation(&context_user, capture_id)
        .await?;

    let captures = state.user_api.get_captures(&context_user, vec![capture_id]).await?;
    let capture = captures.into_iter().next().ok_or_else(|| {
        api::ApiError::not_found(anyhow!("Capture with id {} not found", capture_id))
    })?;

    let mut context = state.template_context();
    context.insert("capture", &capture);

    let rendered = state
        .tera
        .render("partials/cards/annotation_block.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}