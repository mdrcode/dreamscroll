mod webhook_state;
pub use webhook_state::*;

mod maker;
pub use maker::*;

pub mod localclient;

pub mod r_illuminate;
pub mod r_search_index;
pub mod r_spark;

use axum::http::StatusCode as HttpStatusCode;

use crate::task::StatusCode;

/// Map a finished attempt's status to the HTTP status Cloud Tasks should see.
///
/// Cloud Tasks retries on any non-2xx response and stops on any 2xx. The app's
/// retry budget is configured to be strictly smaller than the queue's, so the
/// app always exhausts first — meaning both terminal outcomes are acked with a
/// 2xx. We use distinct codes so Cloud Run logs can distinguish them:
///
/// - `CompleteSuccess` -> `204 No Content`      (acked, succeeded)
/// - `CompleteFailure` -> `200 OK`              (acked, gave up; app budget spent)
/// - `ErrorWillRetry`  -> `500 Internal Server Error` (Cloud Tasks should retry)
///
/// NOTE: `ErrorWillRetry` deliberately uses `500`, not `503`. Cloud Tasks treats
/// `503` (and `429`) as *system* errors and responds by throttling the whole
/// queue's dispatch rate, which is a queue-wide side effect we don't want from
/// an ordinary per-task failure. `500` retries the task without that.
///
/// NOTE: this mapping is only visible in Cloud Run *request logs*. Cloud Tasks'
/// own `lastAttempt.responseStatus` is a `google.rpc.Status`, where every 2xx
/// normalizes to `OK`, so it cannot distinguish the two acked cases.
pub fn http_status_for_outcome(status: StatusCode) -> HttpStatusCode {
    match status {
        StatusCode::CompleteSuccess => HttpStatusCode::NO_CONTENT,
        StatusCode::CompleteFailure => HttpStatusCode::OK,
        StatusCode::ErrorWillRetry => HttpStatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::Queued | StatusCode::InProgress => {
            // A finished attempt is never left in a non-terminal state; treat it
            // as a retryable server error rather than silently acking it.
            HttpStatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_retryable_outcomes_return_non_2xx() {
        assert!(http_status_for_outcome(StatusCode::CompleteSuccess).is_success());
        assert!(http_status_for_outcome(StatusCode::CompleteFailure).is_success());
        assert!(
            !http_status_for_outcome(StatusCode::ErrorWillRetry).is_success(),
            "a retryable failure must be non-2xx so Cloud Tasks retries it"
        );
    }

    #[test]
    fn acked_outcomes_use_distinct_status_codes() {
        // Kept distinct so Cloud Run request logs can tell them apart.
        assert_ne!(
            http_status_for_outcome(StatusCode::CompleteSuccess),
            http_status_for_outcome(StatusCode::CompleteFailure)
        );
    }
}
