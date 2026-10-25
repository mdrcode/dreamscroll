use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use axum_login::AuthSession;
use futures_util::StreamExt;
use serde::Deserialize;

use crate::{api, auth, sse};

use super::WebState;

// Clients cannot observe SSE keep-alive comments, so send a named liveness event.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(3 * 60);
// A blended feed renders at most 100 cards; each ID becomes a bind parameter.
const MAX_CATCHUP_ENTITIES: usize = 200;

#[derive(Debug, Deserialize)]
pub struct CatchupParams {
    // `StatusCatchup::events` is generic over entity type; this query parameter
    // is capture-only until another entity needs catch-up. Expanding it
    // (for example, URL-encoded JSON like
    // `catchup=[{"entity_type":"capture","ids":[5,6,7]}]`) is premature until then.
    capture_ids: Option<String>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(params): Query<CatchupParams>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, api::ApiError> {
    let user = auth
        .user
        .expect("protected route; user must be authenticated");

    // Immediately subscribe so we don't miss events while doing catch-up
    let server_events_rx = state.server_events.subscribe();

    let catchup_capture_ids = parse_capture_ids(params.capture_ids.as_deref())?;
    let catchup_events = if catchup_capture_ids.is_empty() {
        None
    } else {
        catchup_or_skip(
            state
                .status_catchup
                .events(user.user_id(), "capture", &catchup_capture_ids)
                .await,
        )
    };

    let event_stream = sse::http::make_sse_stream(
        user.user_id(),
        catchup_events,
        server_events_rx,
        state.shutdown.clone(),
        tokio::time::Instant::now() + MAX_STREAM_LIFETIME, // deadline
        HEARTBEAT_INTERVAL,
    )
    .map(|item| Ok::<_, Infallible>(item.into()));

    Ok(Sse::new(event_stream))
}

/// A failed catch-up must not fail the stream.
///
/// EventSource does not retry a non-200 response, so returning an error here
/// would close the page's only source of live status until the user interacts.
/// Live-only delivery is strictly better than a dead stream.
fn catchup_or_skip(
    result: Result<Vec<sse::TaskStatusUpdate>, api::ApiError>,
) -> Option<Vec<sse::TaskStatusUpdate>> {
    match result {
        Ok(events) => Some(events),
        Err(error) => {
            tracing::warn!(
                error = ?error,
                "Status catch-up failed; continuing with live events only"
            );
            None
        }
    }
}

fn parse_capture_ids(raw: Option<&str>) -> Result<Vec<i32>, api::ApiError> {
    let Some(raw) = raw.filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };

    let mut ids = Vec::new();
    for value in raw.split(',') {
        let id = value.parse::<i32>().map_err(|error| {
            api::ApiError::bad_request(anyhow::anyhow!(
                "invalid capture_ids value {value:?}: {error}"
            ))
        })?;
        if id <= 0 {
            return Err(api::ApiError::bad_request(anyhow::anyhow!(
                "capture_ids must contain positive IDs"
            )));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
        if ids.len() > MAX_CATCHUP_ENTITIES {
            return Err(api::ApiError::bad_request(anyhow::anyhow!(
                "capture_ids accepts at most {MAX_CATCHUP_ENTITIES} IDs"
            )));
        }
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_catchup_degrades_to_live_only() {
        assert!(
            catchup_or_skip(Err(api::ApiError::internal(anyhow::anyhow!(
                "pool timeout"
            ))))
            .is_none(),
            "a non-200 response closes EventSource, so catch-up failure must not fail the stream"
        );

        let row = crate::model::task_run::task_run_model();
        let event = sse::TaskStatusUpdate::from_row(&row, None).unwrap();
        let events = catchup_or_skip(Ok(vec![event])).expect("successful catch-up is forwarded");
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn capture_ids_are_capped() {
        let at_limit = (1..=MAX_CATCHUP_ENTITIES as i32)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse_capture_ids(Some(&at_limit)).unwrap().len(),
            MAX_CATCHUP_ENTITIES
        );

        let over_limit = (1..=MAX_CATCHUP_ENTITIES as i32 + 1)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            parse_capture_ids(Some(&over_limit)).is_err(),
            "an unbounded list becomes unbounded bind parameters"
        );
    }

    #[test]
    fn capture_ids_are_optional_deduplicated_and_positive() {
        assert_eq!(parse_capture_ids(None).unwrap(), Vec::<i32>::new());
        assert_eq!(parse_capture_ids(Some("")).unwrap(), Vec::<i32>::new());
        assert_eq!(parse_capture_ids(Some("12,7,12")).unwrap(), vec![12, 7]);
        assert!(parse_capture_ids(Some("12,nope")).is_err());
        assert!(parse_capture_ids(Some("0")).is_err());
        assert!(parse_capture_ids(Some("12,")).is_err());
    }

    #[test]
    fn catchup_row_becomes_a_task_status_event() {
        let mut row = crate::model::task_run::task_run_model();
        row.entity_owner_id = 7;
        row.logical_id = "u7-illuminate-capture42".to_string();
        row.run_id = "720196b8-3dbf-4c63-9b78-c3f05787d831".to_string();
        row.run_number = 3;
        row.status_code = crate::task::TaskRunStatus::InProgress.as_i32();
        row.attempts = 2;

        let estimate = crate::task::timing::TaskTimingEstimate {
            task_type: "illuminate".to_string(),
            sample_count: crate::task::timing::MINIMUM_CLIENT_ESTIMATE_SAMPLES,
            duration_ms_avg: 10_000,
            duration_ms_p50: 10_000,
            duration_ms_p75: 12_000,
            duration_ms_p90: 15_000,
        };
        let event = sse::TaskStatusUpdate::from_row(&row, Some(&estimate)).unwrap();
        assert_eq!(event.entity_id, 42);
        assert_eq!(event.payload.status, crate::task::TaskRunStatus::InProgress);
        assert_eq!(event.payload.run_number, 3);
        assert_eq!(event.entity_owner_id, Some(7));
        assert_eq!(event.payload.estimated_duration_ms_p50, Some(10_000));
    }

    #[test]
    fn catchup_row_with_unknown_status_is_skipped() {
        let mut row = crate::model::task_run::task_run_model();
        row.id = 2;
        row.entity_owner_id = 7;
        row.logical_id = "u7-illuminate-capture42".to_string();
        row.run_id = "69d394f8-5caa-4363-8c3e-d081b4a353a5".to_string();
        row.status_code = i32::MAX;

        assert!(sse::TaskStatusUpdate::from_row(&row, None).is_none());
    }
}
