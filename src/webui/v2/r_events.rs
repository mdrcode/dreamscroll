use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use axum_login::AuthSession;
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{api, auth, sse};

use super::WebState;

// Clients cannot observe SSE keep-alive comments, so send a named liveness event.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(3 * 60);
// A blended feed renders at most 100 cards; anything past this is not a page
// asking for its own catch-up. Each ID becomes a bind parameter.
const MAX_CATCHUP_CAPTURE_IDS: usize = 200;

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
    let context = auth::Context::from(
        auth.user
            .expect("protected route requires an authenticated user"),
    );
    let server_events_rx = state.server_events.subscribe();
    let catchup_capture_ids = parse_capture_ids(params.capture_ids.as_deref())?;
    let catchup_events = if catchup_capture_ids.is_empty() {
        None
    } else {
        catchup_or_skip(
            state
                .status_catchup
                .events(context.user_id(), "capture", &catchup_capture_ids)
                .await,
        )
    };

    let task_status_stream = make_task_status_stream(
        context.user_id(),
        catchup_events,
        server_events_rx,
        state.shutdown.clone(),
        tokio::time::Instant::now() + MAX_STREAM_LIFETIME, // deadline
        HEARTBEAT_INTERVAL,
    )
    .map(|item| Ok(task_status_sse_event(item)));

    Ok(Sse::new(task_status_stream))
}

#[derive(Debug, Eq, PartialEq)]
enum TaskStatusStreamItem {
    TaskStatus(sse::TaskStatusEvent),
    Heartbeat,
}

fn task_status_sse_event(item: TaskStatusStreamItem) -> Event {
    match item {
        TaskStatusStreamItem::TaskStatus(task_status) => Event::default()
            .event("task-status")
            .data(task_event_json(&task_status)),
        // Axum omits empty data fields, and EventSource does not dispatch data-less events.
        TaskStatusStreamItem::Heartbeat => Event::default().event("heartbeat").data("1"),
    }
}

/// A failed catch-up must not fail the stream.
///
/// EventSource does not retry a non-200 response, so returning an error here
/// would close the page's only source of live status until the user interacts.
/// Live-only delivery is strictly better than a dead stream.
fn catchup_or_skip(
    result: Result<Vec<sse::TaskStatusEvent>, api::ApiError>,
) -> Option<Vec<sse::TaskStatusEvent>> {
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

fn make_task_status_stream(
    user_id: i32,
    catchup: Option<Vec<sse::TaskStatusEvent>>,
    events_rx: broadcast::Receiver<sse::ReceivedServerEvent>,
    shutdown: tokio::sync::watch::Receiver<bool>,
    deadline: tokio::time::Instant,
    heartbeat_interval: Duration,
) -> impl futures_util::Stream<Item = TaskStatusStreamItem> {
    let next_heartbeat = tokio::time::Instant::now() + heartbeat_interval;
    let live_events = stream::unfold(
        (events_rx, user_id, shutdown, deadline, next_heartbeat),
        move |(mut events_rx, user_id, mut shutdown, deadline, mut next_heartbeat)| async move {
            loop {
                if *shutdown.borrow() {
                    return None;
                }

                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => return None,
                    _ = tokio::time::sleep_until(next_heartbeat) => {
                        next_heartbeat = tokio::time::Instant::now() + heartbeat_interval;
                        return Some((
                            TaskStatusStreamItem::Heartbeat,
                            (events_rx, user_id, shutdown, deadline, next_heartbeat),
                        ));
                    }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return None;
                        }
                    }
                    received = events_rx.recv() => {
                        match received {
                            Ok(received) => {
                                if let Some(update) = filter_for_user(received, user_id) {
                                    return Some((
                                        TaskStatusStreamItem::TaskStatus(update),
                                        (events_rx, user_id, shutdown, deadline, next_heartbeat),
                                    ));
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                // Dropped frames may include a terminal status, which
                                // would strand the client's progress UI. Ending the
                                // stream makes the browser reconnect in seconds and
                                // replay the current state through catch-up.
                                tracing::warn!(
                                    skipped,
                                    "SSE receiver lagged; reconnecting so catch-up restores state"
                                );
                                return None;
                            }
                            Err(broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                }
            }
        },
    );
    let catchup_events = stream::iter(
        catchup
            .into_iter()
            .flatten()
            .map(TaskStatusStreamItem::TaskStatus),
    );
    catchup_events.chain(live_events)
}

fn filter_for_user(
    received: sse::ReceivedServerEvent,
    user_id: i32,
) -> Option<sse::TaskStatusEvent> {
    match received {
        sse::ReceivedServerEvent::TaskStatus(update) if update.user_id == Some(user_id) => {
            Some(update)
        }
        sse::ReceivedServerEvent::TaskStatus(_) | sse::ReceivedServerEvent::Availability(_) => None,
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
        if ids.len() > MAX_CATCHUP_CAPTURE_IDS {
            return Err(api::ApiError::bad_request(anyhow::anyhow!(
                "capture_ids accepts at most {MAX_CATCHUP_CAPTURE_IDS} IDs"
            )));
        }
    }

    Ok(ids)
}

fn task_event_json(update: &sse::TaskStatusEvent) -> String {
    let mut data = serde_json::to_value(update).unwrap_or_default();
    if let Some(object) = data.as_object_mut() {
        object.remove("user_id");
    }
    serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use futures_util::StreamExt;
    use tokio::sync::{broadcast, watch};

    use super::*;

    #[test]
    fn task_status_sse_omits_internal_user_id() {
        let update = sse::ServerEvent::<sse::TaskStatusPayload>::new(
            sse::ServerEventTypes::TaskStatus,
            Utc::now(),
            "capture",
            42,
            sse::TaskStatusPayload {
                task_type: "illuminate".to_string(),
                status: crate::task::TaskRunStatus::InProgress,
                logical_id: "u77-illuminate-capture42".to_string(),
                run_id: "720196b8-3dbf-4c63-9b78-c3f05787d831".to_string(),
                run_number: 2,
                attempts: 1,
                processing_started_at: None,
                estimated_duration_ms_p50: None,
            },
        );
        let update = sse::ServerEvent {
            user_id: Some(77),
            ..update
        };

        let event_json = task_event_json(&update);
        assert!(!event_json.contains("user_id"));
        assert!(event_json.contains("\"entity_id\":42"));
        assert!(event_json.contains("\"event_type\":\"task_status\""));
    }

    #[test]
    fn a_failed_catchup_degrades_to_live_only() {
        assert!(
            catchup_or_skip(Err(api::ApiError::internal(anyhow::anyhow!(
                "pool timeout"
            ))))
            .is_none(),
            "a non-200 response closes EventSource, so catch-up failure must not fail the stream"
        );
        let events = catchup_or_skip(Ok(vec![task_status_event(
            7,
            42,
            crate::task::TaskRunStatus::InProgress,
        )]))
        .expect("successful catch-up is forwarded");
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn a_lagged_receiver_ends_the_stream_so_catchup_can_restore_state() {
        let (sender, receiver) = broadcast::channel(1);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        // Overflow the one-slot buffer before the stream polls: the receiver
        // reports `Lagged` and a dropped terminal status could strand the UI.
        sender
            .send(sse::ReceivedServerEvent::TaskStatus(task_status_event(
                7,
                42,
                crate::task::TaskRunStatus::InProgress,
            )))
            .unwrap();
        sender
            .send(sse::ReceivedServerEvent::TaskStatus(task_status_event(
                7,
                42,
                crate::task::TaskRunStatus::CompleteSuccess,
            )))
            .unwrap();

        let mut events = Box::pin(make_task_status_stream(
            7,
            None,
            receiver,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_secs(20),
        ));

        assert!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("lagged stream should end promptly")
                .is_none()
        );
    }

    #[test]
    fn capture_ids_are_capped() {
        let at_limit = (1..=MAX_CATCHUP_CAPTURE_IDS as i32)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse_capture_ids(Some(&at_limit)).unwrap().len(),
            MAX_CATCHUP_CAPTURE_IDS
        );

        let over_limit = (1..=MAX_CATCHUP_CAPTURE_IDS as i32 + 1)
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
            measure: crate::task::timing::Measure::ProcessingSuccessful,
            sample_count: crate::task::timing::MINIMUM_CLIENT_ESTIMATE_SAMPLES,
            duration_ms_avg: 10_000,
            duration_ms_p50: 10_000,
            duration_ms_p75: 12_000,
            duration_ms_p90: 15_000,
        };
        let event = sse::TaskStatusEvent::from_row(&row, Some(&estimate)).unwrap();
        assert_eq!(event.entity_id, 42);
        assert_eq!(event.payload.status, crate::task::TaskRunStatus::InProgress);
        assert_eq!(event.payload.run_number, 3);
        assert_eq!(event.user_id, Some(7));
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

        assert!(sse::TaskStatusEvent::from_row(&row, None).is_none());
    }

    #[test]
    fn live_stream_forwards_only_the_authenticated_users_task_status() {
        let own = task_status_event(7, 42, crate::task::TaskRunStatus::InProgress);
        let other = task_status_event(8, 43, crate::task::TaskRunStatus::InProgress);

        assert!(filter_for_user(sse::ReceivedServerEvent::TaskStatus(own), 7).is_some());
        assert!(filter_for_user(sse::ReceivedServerEvent::TaskStatus(other), 7).is_none());
        assert!(
            filter_for_user(
                sse::ReceivedServerEvent::Availability(sse::AvailabilityEvent::new(
                    sse::ServerEventTypes::Availability,
                    Utc::now(),
                    "capture",
                    42,
                    sse::AvailabilityPayload {
                        operation: sse::AvailabilityState::Available,
                    },
                )),
                7,
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn stream_emits_catchup_then_user_wide_live_events() {
        let catchup = task_status_event(7, 42, crate::task::TaskRunStatus::InProgress);
        let (sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_task_status_stream(
            7,
            Some(vec![catchup.clone()]),
            receiver,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_secs(20),
        ));

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("catch-up should be immediate")
                .unwrap(),
            TaskStatusStreamItem::TaskStatus(catchup),
            "catch-up must be the first event"
        );

        // Live delivery is user-wide, not limited to the initial entity IDs.
        sender
            .send(sse::ReceivedServerEvent::TaskStatus(task_status_event(
                7,
                999,
                crate::task::TaskRunStatus::CompleteSuccess,
            )))
            .unwrap();
        let live = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("live event should follow the catch-up")
            .unwrap();
        let TaskStatusStreamItem::TaskStatus(live) = live else {
            panic!("expected live task-status event");
        };
        assert_eq!(live.entity_id, 999);
        assert_eq!(
            live.payload.status,
            crate::task::TaskRunStatus::CompleteSuccess
        );
    }

    #[tokio::test]
    async fn shutdown_ends_a_stream_even_before_first_event() {
        let (_sender, receiver) = broadcast::channel(8);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_task_status_stream(
            7,
            None,
            receiver,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_secs(20),
        ));

        shutdown_sender.send(true).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("shutdown should end stream promptly")
                .is_none()
        );
    }
    #[tokio::test]
    async fn live_stream_emits_heartbeat_without_task_events() {
        let (_sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_task_status_stream(
            7,
            None,
            receiver,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_millis(10),
        ));

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("heartbeat should arrive on an otherwise idle stream"),
            Some(TaskStatusStreamItem::Heartbeat)
        );
    }

    #[tokio::test]
    async fn heartbeat_sse_frame_includes_a_data_field_for_dispatch() {
        use axum::response::IntoResponse;

        let response = Sse::new(stream::iter([Ok::<_, Infallible>(task_status_sse_event(
            TaskStatusStreamItem::Heartbeat,
        ))]))
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let frame = String::from_utf8(body.to_vec()).unwrap();

        assert!(frame.lines().any(|line| line == "event: heartbeat"));
        assert!(frame.lines().any(|line| line == "data: 1"));
    }

    #[tokio::test]
    async fn server_deadline_closes_the_stream_normally() {
        let (_sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let mut events = Box::pin(make_task_status_stream(
            7,
            None,
            receiver,
            shutdown_receiver,
            deadline,
            Duration::from_secs(20),
        ));

        assert!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("stream should close at its deadline")
                .is_none()
        );
    }

    fn task_status_event(
        user_id: i32,
        entity_id: i32,
        status: crate::task::TaskRunStatus,
    ) -> sse::TaskStatusEvent {
        sse::ServerEvent {
            user_id: Some(user_id),
            ..sse::ServerEvent::new(
                sse::ServerEventTypes::TaskStatus,
                Utc::now(),
                "capture",
                entity_id,
                sse::TaskStatusPayload {
                    task_type: "illuminate".to_string(),
                    status,
                    attempts: 1,
                    logical_id: format!("u{user_id}-illuminate-capture{entity_id}"),
                    run_id: uuid::Uuid::new_v4().to_string(),
                    run_number: 1,
                    processing_started_at: None,
                    estimated_duration_ms_p50: None,
                },
            )
        }
    }
}
