//! HTTP delivery for server-sent events.
//!
//! [`super`] models what the server announces and how an announcement reaches a
//! process; this module turns that into the frames a browser receives over HTTP:
//! the `task-status`/`heartbeat` frame names, heartbeat cadence, stream lifetime,
//! and shutdown.

use std::time::Duration;

use axum::response::sse::Event;
use futures_util::{Stream, StreamExt, stream};
use tokio::sync::{broadcast, watch};

use super::TaskStatusEvent;

/// One frame of the HTTP SSE stream: a payload or a liveness tick.
///
/// Axum's `Event` cannot express "this frame is a heartbeat rather than a task
/// status", so the stream vocabulary lives here, next to the framing.
#[derive(Debug, Eq, PartialEq)]
pub enum HttpStreamItem {
    TaskStatus(TaskStatusEvent),
    Heartbeat,
}

impl HttpStreamItem {
    /// Encode this item as an SSE wire frame.
    pub fn into_wire_frame(self) -> Event {
        match self {
            Self::TaskStatus(task_status) => Event::default().event("task-status").data(
                task_status.browser_json().unwrap_or_else(|error| {
                    tracing::error!(error = ?error, "Failed to serialize task-status event");
                    "{}".to_string()
                }),
            ),
            // Axum omits empty data fields, and EventSource does not dispatch
            // data-less events.
            Self::Heartbeat => Event::default().event("heartbeat").data("1"),
        }
    }
}

pub fn make_event_stream(
    user_id: i32,
    catchup: Option<Vec<TaskStatusEvent>>,
    events_rx: broadcast::Receiver<TaskStatusEvent>,
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
                                if event.belongs_to(user_id) {
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
    use crate::sse::{ServerEvent, ServerEventTypes, TaskStatusPayload};
    use crate::task::TaskRunStatus;

    #[tokio::test]
    async fn stream_emits_catchup_then_user_wide_live_events() {
        let catchup = task_status_event(7, 42, TaskRunStatus::InProgress);
        let (sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(make_event_stream(
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

        let mut events = Box::pin(make_event_stream(
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
        let mut events = Box::pin(make_event_stream(
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
        let mut events = Box::pin(make_event_stream(
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
            HttpStreamItem::Heartbeat.into_wire_frame(),
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
        let mut events = Box::pin(make_event_stream(
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

    fn task_status_event(user_id: i32, entity_id: i32, status: TaskRunStatus) -> TaskStatusEvent {
        ServerEvent {
            user_id: Some(user_id),
            ..ServerEvent::new(
                ServerEventTypes::TaskStatus,
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
