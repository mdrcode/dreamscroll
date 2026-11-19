use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
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

    let entity = match state.user_api.get_entity(&context_user, id).await {
        Ok(entity) => entity,
        Err(error) if error.http_status_code == axum::http::StatusCode::NOT_FOUND => {
            match state
                .user_api
                .superseded_capture_id(&context_user, id)
                .await?
            {
                Some(capture_id) => {
                    return Ok(Redirect::to(&format!("/capture/{capture_id}")).into_response());
                }
                None => return Err(error),
            }
        }
        Err(error) => return Err(error),
    };
    // All readings of the description, the entity's current one first. The observation's own
    // wording is appended when the projection does not already carry it.
    let descriptions = match &entity.knowledge_entity {
        Some(known) => {
            let mut descriptions = Vec::new();
            if let Some(description) = &known.description {
                descriptions.push(description.clone());
            }
            for description in &known.alt_descriptions {
                if !descriptions.contains(description) {
                    descriptions.push(description.clone());
                }
            }
            if let Some(description) = &entity.description {
                if !descriptions.contains(description) {
                    descriptions.push(description.clone());
                }
            }
            descriptions
        }
        None => Vec::new(),
    };
    let occurrences = match &entity.knowledge_entity {
        Some(known) => {
            state
                .user_api
                .get_entity_occurrences(&context_user, known.id, entity.id)
                .await?
        }
        None => Vec::new(),
    };
    let capture = state
        .user_api
        .get_capture(&context_user, entity.capture_id)
        .await?;

    let siblings: Vec<api::ObservedEntityInfo> = capture
        .illuminations
        .first()
        .map(|illumination| {
            illumination
                .observed_entities
                .iter()
                .filter(|sibling| sibling.id != entity.id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let mut context = state.template_context();
    context.insert("page_kind", &"entity_detail");
    context.insert("entity", &entity);
    context.insert("descriptions", &descriptions);
    context.insert("capture", &capture);
    context.insert("occurrences", &occurrences);
    context.insert("siblings", &siblings);

    let rendered = state.render("entity.html.tera", &context)?;

    Ok(Html(rendered).into_response())
}
