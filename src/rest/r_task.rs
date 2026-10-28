use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::{api, auth};

use super::RestState;

/// GET /api/tasks/{run_id}
pub async fn get_run(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(run_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let status = state.user_api.get_task_run(&user.into(), &run_id).await?;
    Ok(Json(status))
}
