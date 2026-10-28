use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};

use crate::{api, auth, stats};

use super::{RestState, query::HoursQuery};

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
