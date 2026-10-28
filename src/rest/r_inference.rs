use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};

use crate::{api, auth, stats};

use super::{RestState, query::HoursQuery};

/// GET /api/admin/inference/{task_run_id} - full inference metadata per attempt (admin only).
pub async fn meta(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(task_run_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let metas = state
        .admin_api
        .inference_meta(&user.into(), &task_run_id)
        .await?;

    Ok(Json(metas))
}

/// GET /api/admin/inference/stats?hours=24 - windowed inference duration/tokens (admin only).
pub async fn stats(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Query(query): Query<HoursQuery>,
) -> Result<impl IntoResponse, api::ApiError> {
    let windows = stats::parse_windows(query.hours.as_deref())?;
    let response = state
        .admin_api
        .inference_stats(&user.into(), &windows)
        .await?;

    Ok(Json(response))
}
