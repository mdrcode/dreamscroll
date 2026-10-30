use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct EntityQuery {
    #[serde(rename = "type")]
    pub entity_type: String,
}

/// GET /api/entity/{id} - Fetch one of the authenticated user's entities.
///
/// Scoped to the token's user: another user's entity id returns 404, as does an
/// entity whose capture is archived.
///
/// Requires JWT authentication.
pub async fn get_by_id(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(entity_id): Path<i32>,
) -> Result<impl IntoResponse, api::ApiError> {
    let entity = state.user_api.get_entity(&user.into(), entity_id).await?;

    Ok(Json(entity))
}

/// GET /api/entity?type=unknown - List the authenticated user's entities of a type.
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<EntityQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let entities = state
        .user_api
        .get_entities_by_type(&user.into(), &query.entity_type)
        .await?;

    Ok(Json(entities))
}
