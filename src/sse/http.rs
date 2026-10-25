//! HTTP delivery for server-sent events.
//!
//! [`super`] models what the server announces and how an announcement reaches a
//! process; this module turns that into the frames a browser receives over HTTP:
//! the `task-status`/`heartbeat` frame names, heartbeat cadence, stream lifetime,
//! and shutdown.

use std::time::Duration;

use axum::response::sse;
use futures_util::{Stream, StreamExt, stream};
use tokio::sync::{broadcast, watch};

use super::TaskStatusUpdate;

/// One frame of the HTTP SSE stream: a payload or a liveness tick.
///
/// Axum's `Event` cannot express "this frame is a heartbeat rather than a task
/// status", so the stream vocabulary lives here, next to the framing.
#[derive(Debug, Eq, PartialEq)]
pub enum HttpStreamItem {
    Heartbeat,
    TaskStatus(TaskStatusUpdate),
}

impl From<HttpStreamItem> for sse::Event {
    fn from(item: HttpStreamItem) -> Self {
        match item {
            HttpStreamItem::TaskStatus(update) => sse::Event::default()
                .event("task-status")
                .data(filtered_json(&update)),
            // Axum omits empty data fields, and EventSource does not dispatch
            // data-less events.
            HttpStreamItem::Heartbeat => sse::Event::default().event("heartbeat").data("1"),
        }
    }
}

/// The SSE payload: the same envelope minus the routing-only `entity_owner_id`.
///
/// One `Serialize` impl serves two destinations, and only `pg_notify` needs the
/// owner (to route across instances). Serde attributes are per type, so the field
/// is cleared here and `skip_serializing_if` omits it.
fn filtered_json(update: &TaskStatusUpdate) -> String {
    let mut filtered = update.clone();
    filtered.entity_owner_id = None;
    serde_json::to_string(&filtered).unwrap_or_else(|error| {
        tracing::error!(error = ?error, "Failed to serialize task-status update");
        "{}".to_string()
    })
}

pub fn make_sse_stream(
    user_id: i32,
    catchup: Option<Vec<TaskStatusUpdate>>,
    events_rx: broadcast::Receiver<TaskStatusUpdate>,
    shutdown: watch::Receiver<bool>,
    deadline: tokio::time::Instant,
    heartbeat_interval: Duration,
) -> impl Stream<Item = HttpStreamItem> {
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
                            HttpStreamItem::Heartbeat,
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
                            Ok(event) => {
                                // Routing: this connection only carries its own user.
                                if event.entity_owner_id == Some(user_id) {
                                    return Some((
                                        HttpStreamItem::TaskStatus(event),
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
            .map(HttpStreamItem::TaskStatus),
    );
    catchup_events.chain(live_events)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use chrono::Utc;
    use futures_util::StreamExt;
    use tokio::sync::{broadcast, watch};

    use super::*;
    use crate::sse::{ServerEventEnvelope, ServerEventKind, TaskStatusPayload};
    use crate::task::TaskRunStatus;

    #[tokio::test]
    async fn stream_emits_catchup_then_user_wide_live_events() {
        let catchup = task_status_event(7, 42, TaskRunStatus::InProgress);
        let (sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_sse_stream(
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
            HttpStreamItem::TaskStatus(catchup),
            "catch-up must be the first event"
        );

        // Live delivery is user-wide, not limited to the initial entity IDs.
        sender
            .send(task_status_event(7, 999, TaskRunStatus::CompleteSuccess))
            .unwrap();
        let live = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("live event should follow the catch-up")
            .unwrap();
        let HttpStreamItem::TaskStatus(live) = live else {
            panic!("expected live task-status event");
        };
        assert_eq!(live.entity_id, 999);
        assert_eq!(live.payload.status, TaskRunStatus::CompleteSuccess);
    }

    #[test]
    fn filtered_payload_omits_the_entity_owner() {
        let update = task_status_event(77, 42, TaskRunStatus::InProgress);

        let json = filtered_json(&update);
        assert!(!json.contains("entity_owner_id"));
        assert!(json.contains("\"entity_id\":42"));
        assert!(json.contains("\"event_type\":\"task_status\""));
        assert!(
            serde_json::to_string(&update)
                .unwrap()
                .contains("entity_owner_id"),
            "the notification form keeps the routing field"
        );
    }

    #[tokio::test]
    async fn another_users_task_status_is_not_forwarded() {
        let (sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_sse_stream(
            7,
            None,
            receiver,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
            Duration::from_secs(20),
        ));

        sender
            .send(task_status_event(8, 43, TaskRunStatus::InProgress))
            .unwrap();
        sender
            .send(task_status_event(7, 44, TaskRunStatus::InProgress))
            .unwrap();

        let item = tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("the recipient's event should arrive")
            .unwrap();
        let HttpStreamItem::TaskStatus(event) = item else {
            panic!("expected a task-status item");
        };
        assert_eq!(
            event.entity_id, 44,
            "the other user's event must be filtered, not just reordered"
        );
    }

    #[tokio::test]
    async fn a_lagged_receiver_ends_the_stream_so_catchup_can_restore_state() {
        let (sender, receiver) = broadcast::channel(1);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        // Overflow the one-slot buffer before the stream polls: the receiver
        // reports `Lagged` and a dropped terminal status could strand the UI.
        sender
            .send(task_status_event(7, 42, TaskRunStatus::InProgress))
            .unwrap();
        sender
            .send(task_status_event(7, 42, TaskRunStatus::CompleteSuccess))
            .unwrap();

        let mut events = Box::pin(make_sse_stream(
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

    #[tokio::test]
    async fn shutdown_ends_a_stream_even_before_first_event() {
        let (_sender, receiver) = broadcast::channel(8);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_sse_stream(
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
        let mut events = Box::pin(make_sse_stream(
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
            Some(HttpStreamItem::Heartbeat)
        );
    }

    #[tokio::test]
    async fn heartbeat_sse_frame_includes_a_data_field_for_dispatch() {
        use axum::response::IntoResponse;

        let response = axum::response::sse::Sse::new(stream::iter([Ok::<_, Infallible>(
            sse::Event::from(HttpStreamItem::Heartbeat),
        )]))
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
        let mut events = Box::pin(make_sse_stream(
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

    fn task_status_event(user_id: i32, entity_id: i32, status: TaskRunStatus) -> TaskStatusUpdate {
        ServerEventEnvelope {
            entity_owner_id: Some(user_id),
            ..ServerEventEnvelope::new(
                ServerEventKind::TaskStatus,
                Utc::now(),
                "capture",
                entity_id,
                TaskStatusPayload {
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
