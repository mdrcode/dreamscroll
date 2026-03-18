use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use axum_extra::extract::Query;
use serde::Deserialize;

use crate::{api, auth};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    limit: Option<u64>,
}

/// GET /api/timeline - Fetch captures for the user's timeline
///
/// Returns a JSON array of capture information including associated
/// media and illuminations, ordered by creation date (newest first).
///
/// Query parameters:
/// - `limit` (optional): Maximum number of captures to return
///
/// Requires JWT authentication.
pub async fn get(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<TimelineQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let limit = query.limit.unwrap_or(100);
    let capture_infos = state.user_api.get_timeline(&user.into(), limit).await?;
    tracing::info!(count = capture_infos.len(), "Fetched timeline captures");
    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
