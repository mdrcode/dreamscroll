//! JWT token issuance endpoint.
//!
//! Provides an endpoint for authenticated users to obtain JWT tokens for API access.

use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::{auth::password, common::AppError, entity::user};

use super::ApiState;

/// Request body for token generation.
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub username: String,
    pub password: String,
}

/// Response body containing the JWT token.
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

/// POST /api/token - Obtain a JWT token by providing credentials.
///
/// This endpoint authenticates a user with username/password and returns
/// a JWT token that can be used to access protected API endpoints.
///
/// # Request Body
///
/// ```json
/// {
///     "username": "user@example.com",
///     "password": "secret"
/// }
/// ```
///
/// # Response
///
/// On success:
/// ```json
/// {
///     "access_token": "eyJ...",
///     "token_type": "Bearer",
///     "expires_in": 86400
/// }
/// ```
///
/// On failure: HTTP 401 Unauthorized
#[tracing::instrument(skip(state, request), fields(username = %request.username))]
pub async fn post(
    State(state): State<Arc<ApiState>>,
    Json(request): Json<TokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Look up the user by username
    let user_opt = user::Entity::find_by_username(&request.username)
        .one(&state.db.conn)
        .await?;

    let user = user_opt.ok_or_else(|| {
        tracing::warn!("Login attempt for unknown user");
        AppError::unauthorized(anyhow::anyhow!("Invalid credentials"))
    })?;

    // Verify password
    let password_valid = password::verify(&user.password_hash, &request.password)?;
    if !password_valid {
        tracing::warn!(user_id = user.id, "Login attempt with invalid password");
        return Err(AppError::unauthorized(anyhow::anyhow!(
            "Invalid credentials"
        )));
    }

    // Create JWT token
    let token = state
        .jwt_config
        .create_token(user.id)
        .map_err(|e| AppError::internal(anyhow::anyhow!("Token creation failed: {e}")))?;

    tracing::info!(user_id = user.id, "JWT token issued successfully");

    Ok(Json(TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: state.jwt_config.expiration_secs,
    }))
}
