use std::sync::Arc;

use crate::{api, common::AppError};
use axum::{Json, extract::State, response::IntoResponse};

use super::ApiState;

/// GET /api/timeline - Fetch all captures for the user's timeline
///
/// Returns a JSON array of capture information including associated
/// media and illuminations, ordered by creation date (newest first).
#[tracing::instrument(skip(state))]
pub async fn get(State(state): State<Arc<ApiState>>) -> Result<impl IntoResponse, AppError> {
    let capture_infos = api::fetch_timeline(&state.db).await?;
    tracing::info!(count = capture_infos.len(), "Fetched timeline captures");
    Ok(Json(capture_infos))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
