use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
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
/// - `id` (optional, repeatable): Specific capture IDs to fetch
///
/// Examples:
/// - `GET /api/captures` - returns all captures
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
    let ids = if query.id.is_empty() {
        None
    } else {
        Some(query.id)
    };

    let capture_infos = state.user_api.get_captures(&user.into(), ids).await?;

    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
