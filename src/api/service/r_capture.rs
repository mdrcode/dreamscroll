use std::sync::Arc;

use crate::{api, common::AppError};
use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use super::ApiState;

/// GET /api/capture/{id} - Fetch a capture by ID
///
/// Returns a JSON object containing the capture information including
/// associated media and illuminations.
#[tracing::instrument(skip(state))]
pub async fn get(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<i32>,
) -> Result<impl IntoResponse, AppError> {
    let capture_info = api::fetch_capture_by_id(&state.db, id).await?;
    Ok(Json(capture_info))
}

#[cfg(test)]
mod tests {
    //use super::*;

    // TODO: Add integration tests once test infrastructure is set up
}
