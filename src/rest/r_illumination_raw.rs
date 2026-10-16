use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::{api, auth};

use super::RestState;

/// GET /api/illuminations/raw/{task_run_id}
pub(super) async fn get(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(task_run_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let result = state
        .user_api
        .get_illumination_raw(&user.into(), &task_run_id)
        .await?;

    Ok(Json(result))
}
