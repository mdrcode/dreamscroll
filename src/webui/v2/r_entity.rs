use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Response},
};
use axum_login::AuthSession;

use crate::{api, auth};

use super::WebState;

/// Full HTML occurrence page for one knowledge entity.
pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Path(id): Path<i32>,
) -> Result<Response, api::ApiError> {
    let user = auth.user.unwrap();
    let context_user = user.into();

    let entity = state.user_api.get_entity(&context_user, id).await?;
    let capture = state
        .user_api
        .get_capture(&context_user, entity.capture_id)
        .await?;

    let siblings: Vec<api::KnowledgeEntityInfo> = capture
        .illuminations
        .first()
        .map(|illumination| {
            illumination
                .knowledge_entities
                .iter()
                .filter(|sibling| sibling.id != entity.id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let mut context = state.template_context();
    context.insert("page_kind", &"entity_detail");
    context.insert("entity", &entity);
    context.insert("capture", &capture);
    context.insert("siblings", &siblings);

    let rendered = state.render("entity.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}
