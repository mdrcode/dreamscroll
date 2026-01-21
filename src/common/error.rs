use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub struct AppError {
    status_code: StatusCode,
    error: anyhow::Error,
}

impl AppError {
    pub fn new(code: StatusCode, error: anyhow::Error) -> Self {
        AppError {
            status_code: code,
            error,
        }
    }

    pub fn into_inner(self) -> anyhow::Error {
        self.error
    }

    pub fn bad_request(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error.into())
    }

    pub fn unauthorized(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, error.into())
    }

    pub fn not_found(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::NOT_FOUND, error.into())
    }

    pub fn payload_too_large(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, error.into())
    }

    pub fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.into())
    }
}

impl IntoResponse for AppError {
    // TODO should only show detailed error in dev mode
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self.error);
        (self.status_code, body).into_response()
    }
}

// Allow AppError to be converted back to anyhow::Error
impl From<AppError> for anyhow::Error {
    fn from(err: AppError) -> Self {
        err.error
    }
}

// Specific From implementations for common error types
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err)
    }
}

impl From<sea_orm::DbErr> for AppError {
    fn from(err: sea_orm::DbErr) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}

impl From<crate::auth::AuthError> for AppError {
    fn from(err: crate::auth::AuthError) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!("{}", err))
    }
}
