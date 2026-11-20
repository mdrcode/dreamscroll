use std::sync::Arc;

use axum::{
    extract::{Form, Path, State},
    http::HeaderMap,
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

/// HTMX sends the page URL with each request; the capture detail page is the one surface that
/// always offers re-illumination.
fn is_capture_detail(headers: &HeaderMap) -> bool {
    let Some(current) = headers
        .get("HX-Current-URL")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let path = current
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|index| &rest[index..]))
        .unwrap_or("/");
    path.split('?')
        .next()
        .unwrap_or(path)
        .starts_with("/capture/")
}

fn render_annotation_response(
    state: &WebState,
    capture: &api::CaptureInfo,
    annotation_template: &str,
    annotation_editing: bool,
    always_show_reilluminate: bool,
) -> Result<Response, api::ApiError> {
    let mut context = state.template_context();
    context.insert("capture", capture);
    context.insert("always_show_reilluminate", &always_show_reilluminate);

    let annotation_html = state.render(annotation_template, &context)?;

    context.insert("oob", &true);
    context.insert("annotation_editing", &annotation_editing);
    let footer_html = state.render("partials/cards/annotation_footer_note.html.tera", &context)?;

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
        false,
    )
}

pub async fn block(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    headers: HeaderMap,
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
        is_capture_detail(&headers),
    )
}

pub async fn set(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    headers: HeaderMap,
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
        is_capture_detail(&headers),
    )
}

pub async fn soft_delete(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(capture_id): Path<i32>,
    headers: HeaderMap,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    state
        .user_api
        .soft_delete_annotation(&context_user, capture_id)
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
        is_capture_detail(&headers),
    )
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::HeaderName};

    use super::is_capture_detail;

    fn headers_with_current_url(url: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("hx-current-url"),
            HeaderValue::from_static(url),
        );
        headers
    }

    #[test]
    fn only_the_capture_detail_surface_always_shows_reilluminate() {
        assert!(is_capture_detail(&headers_with_current_url(
            "http://localhost:8080/capture/7"
        )));
        assert!(is_capture_detail(&headers_with_current_url(
            "http://localhost:8080/capture/7?animate=true"
        )));
        assert!(!is_capture_detail(&headers_with_current_url(
            "http://localhost:8080/"
        )));
        assert!(!is_capture_detail(&headers_with_current_url(
            "http://localhost:8080/?q=capture"
        )));
        assert!(!is_capture_detail(&HeaderMap::new()));
    }
}
