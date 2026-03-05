use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Deserialize;

use crate::{api, auth::DreamscrollAuthUser};

use super::RestState;

/// Request body for admin user creation.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub email: String,
}

/// POST /api/admin/users - Create a new user (admin only)
///
/// Requires JWT authentication and admin permissions.
pub async fn post(
    user: DreamscrollAuthUser,
    State(state): State<Arc<RestState>>,
    Json(request): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let admin_client = api::AdminApiClient::new(state.user_api.db.clone(), user.into())?;

    let user_info = admin_client
        .create_user(request.username, request.password, request.email)
        .await?;

    Ok((StatusCode::CREATED, Json(user_info)).into_response())
}
