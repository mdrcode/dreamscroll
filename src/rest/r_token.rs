//! JWT token issuance endpoint.
//!
//! Provides an endpoint for authenticated users to obtain JWT tokens for API access.

use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::{api, auth};

use super::RestState;

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
pub async fn post(
    State(state): State<Arc<RestState>>,
    Json(request): Json<TokenRequest>,
) -> Result<impl IntoResponse, api::ApiError> {
    let auth_user =
        auth::password::authenticate(&state.user_api.db, &request.username, &request.password).await;

    // If authentication fails, return unauthorized error
    let auth_user = match auth_user {
        Ok(user) => user,
        Err(_) => {
            tracing::warn!("Authentication failed for user {}", request.username);
            return Err(api::ApiError::unauthorized(anyhow::anyhow!(
                "Invalid credentials"
            )));
        }
    };

    // Create JWT token
    let user_id = auth_user.user_id();
    let token = state
        .jwt_config
        .create_user_token(auth_user)
        .map_err(|e| api::ApiError::internal(anyhow::anyhow!("Token creation failed: {e}")))?;

    tracing::info!(user_id, "JWT token issued successfully");

    Ok(Json(TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: state.jwt_config.user_expiration_secs(),
    }))
}
