use axum::http::StatusCode;

use crate::task::TaskRunStatus;

/// Map a finished task run to the HTTP status Cloud Tasks should see.
///
/// Cloud Tasks retries non-2xx responses and acknowledges 2xx responses. The
/// task retry policy therefore uses 500 for retryable failures and 2xx for
/// outcomes that should not be retried.
pub fn http_status_for_task_run(status: TaskRunStatus) -> StatusCode {
    match status {
        TaskRunStatus::SubmissionFailed => StatusCode::INTERNAL_SERVER_ERROR,
        TaskRunStatus::CompleteSuccess => StatusCode::NO_CONTENT,
        TaskRunStatus::CompleteFailure => StatusCode::OK,
        TaskRunStatus::ErrorWillRetry => StatusCode::INTERNAL_SERVER_ERROR,
        TaskRunStatus::Queued | TaskRunStatus::InProgress => {
            // A finished attempt is never left in a non-terminal state; retry
            // rather than silently acknowledging an inconsistent outcome.
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_retryable_outcomes_return_non_2xx() {
        assert!(!http_status_for_task_run(TaskRunStatus::SubmissionFailed).is_success());
        assert!(http_status_for_task_run(TaskRunStatus::CompleteSuccess).is_success());
        assert!(http_status_for_task_run(TaskRunStatus::CompleteFailure).is_success());
        assert!(!http_status_for_task_run(TaskRunStatus::ErrorWillRetry).is_success());
    }

    #[test]
    fn acked_outcomes_use_distinct_status_codes() {
        assert_ne!(
            http_status_for_task_run(TaskRunStatus::CompleteSuccess),
            http_status_for_task_run(TaskRunStatus::CompleteFailure)
        );
    }
}
