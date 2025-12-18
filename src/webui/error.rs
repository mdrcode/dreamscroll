use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

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

    pub fn bad_request(error: impl Into<anyhow::Error>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error.into())
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

// Support `?` with any anyhow-compatible error
impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err.into())
    }
}
