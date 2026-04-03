use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};

use crate::{api, auth};

use super::RestState;

/// POST /api/admin/backfill/enqueue - enqueue backfill tasks (admin only)
pub async fn post(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<api::BackfillRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let response = state
        .admin_api
        .enqueue_backfill(&user.into(), request)
        .await?;

    Ok(Json(response))
}
