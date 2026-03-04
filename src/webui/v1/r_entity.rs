use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};

use crate::{api, auth};

use super::WebState;

pub async fn get_knode(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    tracing::debug!(
        "Rendering entity detail for knode {} for user {}",
        id,
        user.id()
    );

    let entity_info = state.user_api.get_knode(&user.into(), id).await?;

    render_entity(&state, entity_info)
}

pub async fn get_social_media(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    tracing::debug!(
        "Rendering entity detail for social_media {} for user {}",
        id,
        user.id()
    );

    let entity_info = state.user_api.get_social_media(&user.into(), id).await?;

    render_entity(&state, entity_info)
}

fn render_entity(
    state: &WebState,
    entity_info: api::EntityInfo,
) -> Result<Response, api::ApiError> {
    let mut context = state.template_context();
    context.insert("entity", &entity_info);

    let rendered = state
        .tera
        .render("entity.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
