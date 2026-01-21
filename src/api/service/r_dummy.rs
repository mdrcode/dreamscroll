use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::auth::{Context, DreamscrollAuthUser};
use crate::common::AppError;

use super::ApiState;

/// GET /api/dummy - A simple authenticated endpoint for testing.
///
/// This endpoint requires a valid JWT token in the Authorization header.
/// Returns information about the authenticated user.
pub async fn get(
    user: DreamscrollAuthUser,
    State(_state): State<Arc<ApiState>>,
) -> Result<impl IntoResponse, AppError> {
    // Convert the JWT user to a Context for business logic
    let user_context = Context::from(&user);

    Ok(Json(serde_json::json!({
        "message": "Hello from dreamscroll API!",
        "user_id": user_context.user_id(),
    })))
}
