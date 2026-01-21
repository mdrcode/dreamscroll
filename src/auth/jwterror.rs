use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Errors that can occur during JWT operations.
#[derive(Debug)]
pub enum JwtError {
    /// The Authorization header is missing or malformed
    MissingOrInvalidHeader,
    /// The token signature is invalid or the token is expired
    InvalidToken,
    /// An error occurred during token creation
    TokenCreation(String),
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JwtError::MissingOrInvalidHeader => {
                write!(f, "Missing or invalid Authorization header")
            }
            JwtError::InvalidToken => write!(f, "Invalid or expired token"),
            JwtError::TokenCreation(msg) => write!(f, "Token creation failed: {msg}"),
        }
    }
}

impl std::error::Error for JwtError {}

impl From<jsonwebtoken::errors::Error> for JwtError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        use jsonwebtoken::errors::ErrorKind;
        match err.kind() {
            ErrorKind::InvalidToken
            | ErrorKind::InvalidSignature
            | ErrorKind::ExpiredSignature
            | ErrorKind::ImmatureSignature
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_)
            | ErrorKind::Utf8(_) => JwtError::InvalidToken,
            _ => JwtError::TokenCreation(err.to_string()),
        }
    }
}

impl IntoResponse for JwtError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            JwtError::MissingOrInvalidHeader => (StatusCode::UNAUTHORIZED, self.to_string()),
            JwtError::InvalidToken => (StatusCode::UNAUTHORIZED, self.to_string()),
            JwtError::TokenCreation(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
