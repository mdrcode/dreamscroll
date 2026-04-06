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

fn render_annotation_response(
    state: &WebState,
    capture: &api::CaptureInfo,
    annotation_template: &str,
    annotation_editing: bool,
) -> Result<Response, api::ApiError> {
    let mut context = state.template_context();
    context.insert("capture", capture);

    let annotation_html = state
        .tera
        .render(annotation_template, &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    context.insert("oob", &true);
    context.insert("annotation_editing", &annotation_editing);
    let footer_html = state
        .tera
        .render("partials/cards/annotation_footer_left.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(format!("{}{}", annotation_html, footer_html)).into_response())
}

async fn get_capture_or_404(
    state: &WebState,
    context_user: &auth::Context,
    capture_id: i32,
) -> Result<api::CaptureInfo, api::ApiError> {
    let captures = state
        .user_api
        .get_captures(context_user, vec![capture_id])
        .await?;
    captures.into_iter().next().ok_or_else(|| {
        api::ApiError::not_found(anyhow!("Capture with id {} not found", capture_id))
    })
}

pub async fn form(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let capture = get_capture_or_404(&state, &context_user, capture_id).await?;
    render_annotation_response(
        &state,
        &capture,
        "partials/cards/annotation_editor.html.tera",
        true,
    )
}

pub async fn block(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let capture = get_capture_or_404(&state, &context_user, capture_id).await?;
    render_annotation_response(
        &state,
        &capture,
        "partials/cards/annotation_block.html.tera",
        false,
    )
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

    let capture = get_capture_or_404(&state, &context_user, capture_id).await?;
    render_annotation_response(
        &state,
        &capture,
        "partials/cards/annotation_block.html.tera",
        false,
    )
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

    let capture = get_capture_or_404(&state, &context_user, capture_id).await?;
    render_annotation_response(
        &state,
        &capture,
        "partials/cards/annotation_block.html.tera",
        false,
    )
}
