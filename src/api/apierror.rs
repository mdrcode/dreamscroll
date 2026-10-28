use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::telemetry;

#[derive(Debug)]
pub struct ApiError {
    /// HTTP status this error produces for its caller.
    ///
    /// Soft policy: safe to surface to external users, and the only status
    /// visible in responses.
    pub http_status_code: StatusCode,
    error: anyhow::Error,
    /// Status persisted as `task_runs.last_error_code`: the most specific code
    /// known for the failure (for example an upstream provider's 429),
    /// defaulting to `http_status_code`.
    ///
    /// Soft policy: **internal only**. This can identify an upstream provider
    /// (or an implementation detail) and must not be returned to external
    /// users; log and record it, never expose it.
    inner_status_code: i32,
}

impl ApiError {
    pub fn new(code: StatusCode, error: anyhow::Error) -> Self {
        ApiError {
            http_status_code: code,
            error,
            inner_status_code: i32::from(code.as_u16()),
        }
    }

    /// Use a more specific code than `http_status_code` for failure triage (for
    /// example the upstream provider's 429). `None` keeps the default.
    ///
    /// Internal only: do not surface [`ApiError::inner_status_code`] to external
    /// users.
    pub fn with_inner_status_code(mut self, inner_status_code: Option<i32>) -> Self {
        if let Some(inner_status_code) = inner_status_code {
            self.inner_status_code = inner_status_code;
        }
        self
    }

    /// The code persisted as `task_runs.last_error_code`.
    ///
    /// Internal only: for logs, metrics, and failure triage. Never include it in
    /// an API response; external callers see [`ApiError::http_status_code`].
    pub fn inner_status_code(&self) -> i32 {
        self.inner_status_code
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

    /// The failing layer's own message: the annotated error chain on one line,
    /// with `anyhow` context preserved. Persisted for failure triage; it carries
    /// no classification of whose fault the failure was.
    ///
    /// Internal only: may embed upstream response bodies. Log and record it,
    /// never return it to external users.
    pub fn message(&self) -> String {
        format!("{:#}", self.error)
    }

    /// Whether a task that failed with this error is worth retrying.
    ///
    /// Client errors (4xx) are treated as permanent — retrying identical input
    /// will produce identical results — while server errors (5xx) are treated
    /// as transient. See `TaskMaster::finish_attempt`.
    pub fn is_retryable(&self) -> bool {
        self.http_status_code.is_server_error()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let report_id = uuid::Uuid::new_v4().to_string();
        let trace_id = telemetry::current_trace_id();

        tracing::error!(
            status_code = self.http_status_code.as_u16(),
            report_id = %report_id,
            trace_id = trace_id.as_deref().unwrap_or("none"),
            error = ?self.error,
            "ApiError: {:?}", self.error
        );

        let body = match trace_id {
            Some(trace_id) => format!(
                "Request failed. status={} report_id={} trace_id={}",
                self.http_status_code.as_u16(),
                report_id,
                trace_id
            ),
            None => format!(
                "Request failed. status={} report_id={}",
                self.http_status_code.as_u16(),
                report_id
            ),
        };

        (self.http_status_code, body).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_status_code_prefers_the_more_specific_code() {
        let api_error = ApiError::internal(anyhow::anyhow!("Gemini Interactions API returned 429"))
            .with_inner_status_code(Some(429));

        assert_eq!(
            api_error.http_status_code,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(api_error.inner_status_code(), 429);
    }

    #[test]
    fn inner_status_code_defaults_to_http_status_code() {
        let api_error = ApiError::internal(anyhow::anyhow!("boom"));

        assert_eq!(api_error.inner_status_code(), 500);
    }
}
