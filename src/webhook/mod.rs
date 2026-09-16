mod webhook_state;
pub use webhook_state::*;

mod maker;
pub use maker::*;

pub mod localclient;

pub mod r_illuminate;
pub mod r_ingest;
pub mod r_search_index;
pub mod r_spark;

use axum::http::StatusCode as HttpStatusCode;

use crate::task::AttemptOutcome;

/// Map a task attempt outcome to the HTTP status Cloud Tasks should see.
///
/// Cloud Tasks retries on any non-2xx response and stops on any 2xx. The app's
/// retry budget is configured to be strictly smaller than the queue's, so the
/// app always exhausts first — meaning both terminal outcomes are acked with a
/// 2xx. We use distinct codes so Cloud Run logs can distinguish them:
///
/// - `Completed`         -> `204 No Content`      (acked, succeeded)
/// - `ErrorExhausted`    -> `200 OK`              (acked, gave up; app budget spent)
/// - `ErrorWillRetry`    -> `503 Service Unavailable` (Cloud Tasks should retry)
///
/// NOTE: this mapping is only visible in Cloud Run *request logs*. Cloud Tasks'
/// own `lastAttempt.responseStatus` is a `google.rpc.Status`, where every 2xx
/// normalizes to `OK`, so it cannot distinguish the two acked cases.
pub fn http_status_for_outcome(outcome: AttemptOutcome) -> HttpStatusCode {
    match outcome {
        AttemptOutcome::Completed => HttpStatusCode::NO_CONTENT,
        AttemptOutcome::ErrorExhausted => HttpStatusCode::OK,
        AttemptOutcome::ErrorWillRetry => HttpStatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_retryable_outcomes_return_non_2xx() {
        assert!(http_status_for_outcome(AttemptOutcome::Completed).is_success());
        assert!(http_status_for_outcome(AttemptOutcome::ErrorExhausted).is_success());
        assert!(
            !http_status_for_outcome(AttemptOutcome::ErrorWillRetry).is_success(),
            "a retryable failure must be non-2xx so Cloud Tasks retries it"
        );
    }

    #[test]
    fn acked_outcomes_use_distinct_status_codes() {
        // Kept distinct so Cloud Run request logs can tell them apart.
        assert_ne!(
            http_status_for_outcome(AttemptOutcome::Completed),
            http_status_for_outcome(AttemptOutcome::ErrorExhausted)
        );
    }
}
