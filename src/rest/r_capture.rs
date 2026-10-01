use std::sync::Arc;

use anyhow::anyhow;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct CaptureQuery {
    #[serde(default)]
    id: Vec<i32>,
}

/// GET /api/captures - Fetch captures by IDs
///
/// Query parameters:
/// - `id` (required, repeatable): Specific capture IDs to fetch
///
/// Examples:
/// - `GET /api/captures?id=123` - returns capture 123
/// - `GET /api/captures?id=123&id=456&id=789` - returns captures 123, 456, and 789
///
/// Returns a JSON array containing the capture information including
/// associated media and illuminations.
///
/// Requires JWT authentication.
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<CaptureQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    if query.id.is_empty() {
        return Err(api::ApiError::bad_request(anyhow!(
            "At least one id query parameter is required."
        )));
    }

    let capture_infos = state.user_api.get_captures(&user.into(), query.id).await?;

    Ok(Json(capture_infos))
}

/// DELETE /api/captures/{capture_id} - Delete a capture and its associated data
///
/// Requires JWT authentication. The capture must belong to the authenticated user.
pub async fn delete(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
) -> Result<impl IntoResponse, api::ApiError> {
    state
        .user_api
        .delete_capture(&user.into(), capture_id)
        .await?;

    tracing::info!("Deleted capture {} successfully", capture_id);

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/captures/{capture_id}/archive - Archive a capture
///
/// Requires JWT authentication. The capture must belong to the authenticated user.
pub async fn archive(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
) -> Result<impl IntoResponse, api::ApiError> {
    state
        .user_api
        .archive_capture(&user.into(), capture_id)
        .await?;

    tracing::info!("Archived capture {} successfully", capture_id);

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/captures/{capture_id}/unarchive - Unarchive a capture
///
/// Requires JWT authentication. The capture must belong to the authenticated user.
pub async fn unarchive(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
) -> Result<impl IntoResponse, api::ApiError> {
    state
        .user_api
        .unarchive_capture(&user.into(), capture_id)
        .await?;

    tracing::info!("Unarchived capture {} successfully", capture_id);

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
