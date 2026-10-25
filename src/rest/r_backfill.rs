use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::{api, auth, backfill};

use super::RestState;

/// POST /api/admin/backfill/pump - submit the next batch (admin only).
pub async fn pump(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<backfill::BackfillPumpRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let response = state.admin_api.backfill_pump(&user.into(), request).await?;

    Ok(Json(response))
}

/// GET /api/admin/backfill/{backfill_id}/status - task run counts (admin only).
pub async fn status(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(backfill_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let response = state
        .admin_api
        .backfill_status(&user.into(), &backfill_id)
        .await?;

    Ok(Json(response))
}

/// GET /api/admin/backfill/{backfill_id}/failures - terminal failures (admin only).
pub async fn failures(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(backfill_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let response = state
        .admin_api
        .backfill_failures(&user.into(), &backfill_id)
        .await?;

    Ok(Json(response))
}
