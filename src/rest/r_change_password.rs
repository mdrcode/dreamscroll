use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// POST /api/account/password - Change the authenticated user's password
pub async fn post(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let user_id = user.user_id();
    state
        .user_api
        .change_password(&user.into(), request.current_password, request.new_password)
        .await?;

    tracing::info!(user_id, "Changed account password successfully");

    Ok(StatusCode::NO_CONTENT)
}