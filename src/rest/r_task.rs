use std::sync::Arc;

use axum::{Json, extract::{Path, State}, response::IntoResponse};
use serde::Deserialize;

use crate::{api, auth};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct CaptureTaskRequest {
    pub capture_id: i32,
}

/// POST /api/queues/illuminate
pub async fn post_illuminate(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<CaptureTaskRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let identity = state
        .user_api
        .enqueue_illumination(&user.into(), request.capture_id)
        .await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(identity)))
}

/// POST /api/queues/search_index
pub async fn post_search_index(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<CaptureTaskRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let identity = state
        .user_api
        .enqueue_search_index(&user.into(), request.capture_id)
        .await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(identity)))
}

/// GET /api/tasks/{envelope_id}/{run}
pub async fn get_run(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path((envelope_id, run)): Path<(String, i32)>,
) -> Result<impl IntoResponse, api::ApiError> {
    if run < 1 {
        return Err(api::ApiError::bad_request(anyhow::anyhow!(
            "run must be a positive integer"
        )));
    }

    let status = state
        .user_api
        .get_task_run(&user.into(), &envelope_id, run)
        .await?;
    Ok(Json(status))
}
