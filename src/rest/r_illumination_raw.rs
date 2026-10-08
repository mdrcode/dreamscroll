use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};

use crate::{api, auth};

use super::RestState;

/// GET /api/illuminations/raw/{inference_id}
pub(super) async fn get(
    user: auth::DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Path(inference_id): Path<String>,
) -> Result<impl IntoResponse, api::ApiError> {
    let result = state
        .user_api
        .get_illumination_raw(&user.into(), &inference_id)
        .await?;

    Ok(Json(result))
}
