use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug)]
pub struct ApiError {
    status_code: StatusCode,
    error: anyhow::Error,
}

impl ApiError {
    pub fn new(code: StatusCode, error: anyhow::Error) -> Self {
        ApiError {
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

    pub fn conflict(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::CONFLICT, error.into())
    }

    pub fn forbidden(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::FORBIDDEN, error.into())
    }

    pub fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.into())
    }

    pub fn not_found(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::NOT_FOUND, error.into())
    }

    pub fn payload_too_large(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, error.into())
    }

    pub fn unauthorized(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, error.into())
    }
}

impl IntoResponse for ApiError {
    // TODO should only show detailed error in dev mode
    fn into_response(self) -> Response {
        let body = format!("Error: {:?}", self.error);
        (self.status_code, body).into_response()
    }
}

// Allow AppError to be converted back to anyhow::Error
impl From<ApiError> for anyhow::Error {
    fn from(err: ApiError) -> Self {
        err.error
    }
}

// Specific From implementations for common error types
impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err)
    }
}

impl From<sea_orm::DbErr> for ApiError {
    fn from(err: sea_orm::DbErr) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}
