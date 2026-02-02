use std::sync::Arc;

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::{AuthSession, AuthUser};
use tera::Context;

use crate::{api, auth};

use super::WebState;

#[tracing::instrument(skip(auth, state, id))]
pub async fn entity_knode(
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

    Err(api::ApiError::not_found(anyhow!(
        "SocialMedia entity rendering not yet implemented"
    )))

    /*
    let entity_info = api::fetch_knode(&state.db, &user.into(), id)
        .await?
        .ok_or_else(|| api::ApiError::not_found(anyhow!("KNode with id {} not found", id)))?;

    render_entity(&state, entity_info)
    */
}

/*

#[tracing::instrument(skip(auth, state, id))]
pub async fn entity_social_media(
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


    let entity_info = api::fetch_social_media(&state.db, &user.into(), id)
        .await?
        .ok_or_else(|| api::ApiError::not_found(anyhow!("SocialMedia with id {} not found", id)))?;

    render_entity(&state, entity_info)


}


fn render_entity(
    state: &WebState,
    entity_info: api::EntityInfo,
) -> Result<Response, api::ApiError> {
    let mut context = Context::new();
    context.insert("entity", &entity_info);

    let rendered = state
        .tera
        .render("entity.html.tera", &context)
        .map_err(|e| anyhow!("Failed to render template: {:?}", e))?;

    Ok(Html(rendered).into_response())
}
    */
