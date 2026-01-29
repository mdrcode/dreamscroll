use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, auth};

use super::ApiState;

/// GET /api/timeline - Fetch all captures for the user's timeline
///
/// Returns a JSON array of capture information including associated
/// media and illuminations, ordered by creation date (newest first).
///
/// Requires JWT authentication.
#[tracing::instrument(skip(user, state), fields(user_id = %user.user_id()))]
pub async fn get(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<ApiState>>,
) -> Result<impl IntoResponse, api::ApiError> {
    let capture_infos = api::fetch_timeline(&state.db, &user.into()).await?;
    tracing::info!(count = capture_infos.len(), "Fetched timeline captures");
    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
