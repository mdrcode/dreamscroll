use std::sync::Arc;

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

    let annotation_html = state.render(annotation_template, &context)?;

    context.insert("oob", &true);
    context.insert("annotation_editing", &annotation_editing);
    let footer_html = state.render("partials/cards/annotation_footer_left.html.tera", &context)?;

    Ok(Html(format!("{}{}", annotation_html, footer_html)).into_response())
}

pub async fn form(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let capture = state
        .user_api
        .get_capture(&context_user, capture_id)
        .await?;
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

    let capture = state
        .user_api
        .get_capture(&context_user, capture_id)
        .await?;
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

    let capture = state
        .user_api
        .get_capture(&context_user, capture_id)
        .await?;
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

    let capture = state
        .user_api
        .get_capture(&context_user, capture_id)
        .await?;
    render_annotation_response(
        &state,
        &capture,
        "partials/cards/annotation_block.html.tera",
        false,
    )
}
