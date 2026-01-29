use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::ApiState;

#[derive(Debug, Deserialize)]
pub struct CaptureQuery {
    #[serde(default)]
    ids: Vec<i32>,
}

/// GET /api/capture - Fetch captures by IDs
///
/// Query parameters:
/// - `ids` (optional, repeatable): Specific capture IDs to fetch
///
/// Examples:
/// - `GET /api/capture` - returns all captures
/// - `GET /api/capture?ids=123` - returns capture 123
/// - `GET /api/capture?ids=123&ids=456&ids=789` - returns captures 123, 456, and 789
///
/// Returns a JSON array containing the capture information including
/// associated media and illuminations.
///
/// Requires JWT authentication.
#[tracing::instrument(skip(user, state))]
pub async fn get(
    user: DreamscrollAuthUser,
    State(state): State<Arc<ApiState>>,
    Query(query): Query<CaptureQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let ids = if query.ids.is_empty() {
        None
    } else {
        Some(query.ids)
    };

    let capture_infos = api::fetch_captures(&state.db, &user.into(), ids).await?;

    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
