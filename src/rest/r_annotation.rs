use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct SetCaptureAnnotationRequest {
    pub content: String,
}

/// POST /api/captures/{capture_id}/annotation - Create or update active annotation
/// for a capture.
///
/// Request JSON:
/// - `content` (required): Annotation text content
///
/// Requires JWT authentication. The capture must belong to the authenticated user.
pub async fn set(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
    Json(req): Json<SetCaptureAnnotationRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let annotation = state
        .user_api
        .set_annotation(&user.into(), capture_id, req.content)
        .await?;

    Ok(Json(annotation))
}

/// POST /api/captures/{capture_id}/annotation/archive - Archive latest active
/// annotation for a capture.
///
/// Requires JWT authentication. The capture must belong to the authenticated user.
/// Idempotent: returns success even when no active annotation exists.
pub async fn archive(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(capture_id): Path<i32>,
) -> Result<impl IntoResponse, api::ApiError> {
    state
        .user_api
        .archive_annotation(&user.into(), capture_id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
