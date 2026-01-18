use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::common::AppError;

use super::ApiState;

/// GET /api/timeline - Fetch all captures for the user's timeline
///
/// Returns a JSON array of capture information including associated
/// media and illuminations, ordered by creation date (newest first).
pub async fn get(State(_state): State<Arc<ApiState>>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        serde_json::json!({"message": "Hello from dreamscroll API!"}),
    ))
}
