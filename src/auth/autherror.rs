use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub enum AuthError {
    Database(sea_orm::DbErr),

    // Session-specific
    InvalidCredentials,
    PasswordHashError(String),

    // JWT-specific
    MissingOrInvalidHeader,
    InvalidToken,
    TokenCreation(String),

    // Catch-all
    Other(anyhow::Error),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Database(e) => write!(f, "Database error: {}", e),
            AuthError::InvalidCredentials => write!(f, "Invalid password"),
            AuthError::PasswordHashError(e) => write!(f, "Password hash error: {}", e),
            AuthError::MissingOrInvalidHeader => {
                write!(f, "Missing or invalid Authorization header")
            }
            AuthError::InvalidToken => write!(f, "Invalid or expired token"),
            AuthError::TokenCreation(msg) => write!(f, "Token creation failed: {msg}"),
            AuthError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for AuthError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AuthError::Database(e) => Some(e),
            AuthError::Other(e) => e.source(),
            _ => None,
        }
    }
}

impl From<sea_orm::DbErr> for AuthError {
    fn from(e: sea_orm::DbErr) -> Self {
        AuthError::Database(e)
    }
}

impl From<argon2::password_hash::Error> for AuthError {
    fn from(e: argon2::password_hash::Error) -> Self {
        AuthError::PasswordHashError(e.to_string())
    }
}

impl From<jsonwebtoken::errors::Error> for AuthError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        use jsonwebtoken::errors::ErrorKind;
        match err.kind() {
            ErrorKind::InvalidToken
            | ErrorKind::InvalidSignature
            | ErrorKind::ExpiredSignature
            | ErrorKind::ImmatureSignature
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_)
            | ErrorKind::Utf8(_) => AuthError::InvalidToken,
            _ => AuthError::TokenCreation(err.to_string()),
        }
    }
}

impl From<anyhow::Error> for AuthError {
    fn from(e: anyhow::Error) -> Self {
        AuthError::Other(e)
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthError::PasswordHashError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
            AuthError::MissingOrInvalidHeader => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthError::TokenCreation(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            AuthError::Other(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
