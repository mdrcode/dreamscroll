use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, auth, common::AppError};

use super::ApiState;

/// GET /api/timeline - Fetch all captures for the user's timeline
///
/// Returns a JSON array of capture information including associated
/// media and illuminations, ordered by creation date (newest first).
///
/// Requires JWT authentication.
#[tracing::instrument(skip(user, state), fields(user_id = %user.user_id()))]
pub async fn get(
    user: auth::JwtAuthUser,
    State(state): State<Arc<ApiState>>,
) -> Result<impl IntoResponse, AppError> {
    let user_context = auth::Context::from(&user);
    let capture_infos = api::fetch_timeline(user_context, &state.db).await?;
    tracing::info!(count = capture_infos.len(), "Fetched timeline captures");
    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
