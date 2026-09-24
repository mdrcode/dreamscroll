use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use axum_login::AuthSession;
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::{api, auth, sse};

use super::WebState;

#[derive(Debug, Deserialize)]
pub struct EventParams {
    // Catch-up currently supports capture IDs only. Ideally this becomes a
    // generic entity selector (for example, URL-encoded JSON like
    // `catchup=[{"entity_type":"capture","ids":[5,6,7]}]`), but that
    // flexibility is premature until another entity needs catch-up support.
    capture_ids: Option<String>,
}

pub async fn get(
    auth: AuthSession<auth::WebAuthBackend>,
    State(state): State<Arc<WebState>>,
    Query(params): Query<EventParams>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, api::ApiError> {
    let user_id = auth
        .user
        .expect("protected route requires an authenticated user")
        .user_id();
    let receiver = state.server_events.subscribe();
    let shutdown = state.shutdown.clone();
    let capture_ids = parse_capture_ids(params.capture_ids.as_deref())?;
    let snapshot = state
        .task_master
        .query_latest_status_for_entities(user_id, "capture", &capture_ids)
        .await?
        .into_iter()
        .filter_map(task_status_event_from_row)
        .collect::<Vec<_>>();

    let events =
        task_status_stream(snapshot, receiver, user_id, shutdown).map(serialize_task_event);

    Ok(Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("keep-alive"),
    ))
}

fn task_status_stream(
    snapshot: Vec<sse::TaskStatusEvent>,
    receiver: broadcast::Receiver<sse::ReceivedServerEvent>,
    user_id: i32,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> impl futures_util::Stream<Item = sse::TaskStatusEvent> {
    let live_events = stream::unfold(
        (receiver, user_id, shutdown),
        |(mut receiver, user_id, mut shutdown)| async move {
            loop {
                if *shutdown.borrow() {
                    return None;
                }

                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return None;
                        }
                    }
                    received = receiver.recv() => {
                        match received {
                            Ok(received) => {
                                if let Some(update) = task_status_for_user(received, user_id) {
                                    return Some((
                                        update,
                                        (receiver, user_id, shutdown),
                                    ));
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::debug!(
                                    skipped,
                                    "SSE receiver lagged and dropped best-effort updates"
                                );
                            }
                            Err(broadcast::error::RecvError::Closed) => return None,
                        }
                    }
                }
            }
        },
    );
    stream::iter(snapshot).chain(live_events)
}

fn task_status_for_user(
    received: sse::ReceivedServerEvent,
    user_id: i32,
) -> Option<sse::TaskStatusEvent> {
    match received {
        sse::ReceivedServerEvent::TaskStatus(update) if update.payload.user_id == user_id => {
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
    }

    Ok(ids)
}

fn task_status_event_from_row(
    row: crate::model::task_run_status::Model,
) -> Option<sse::TaskStatusEvent> {
    let status = match crate::task::TaskRunStatus::from_i32(row.status_code) {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(
                task_run_status_id = row.id,
                status_code = row.status_code,
                error = ?error,
                "Skipping snapshot row with an unknown task status"
            );
            return None;
        }
    };

    Some(sse::ServerEvent::task_status(
        row.updated_at,
        row.entity_type,
        row.entity_id,
        row.task_type,
        status,
        row.attempts,
        row.run,
        row.user_id,
    ))
}

fn serialize_task_event(update: sse::TaskStatusEvent) -> Result<Event, Infallible> {
    Ok(Event::default()
        .event("task-status")
        .data(task_event_json(&update)))
}

fn task_event_json(update: &sse::TaskStatusEvent) -> String {
    let mut data = serde_json::to_value(&update).unwrap_or_default();
    if let Some(payload) = data
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
    {
        payload.remove("user_id");
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
        let update = sse::ServerEvent::<sse::TaskStatusPayload>::task_status(
            Utc::now(),
            "capture",
            42,
            "illuminate",
            crate::task::TaskRunStatus::InProgress,
            1,
            2,
            77,
        );

        let event_json = task_event_json(&update);
        assert!(!event_json.contains("user_id"));
        assert!(event_json.contains("\"entity_id\":42"));
        assert!(event_json.contains("\"event_type\":\"task_status\""));
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
    fn snapshot_row_becomes_a_task_status_event() {
        let row = crate::model::task_run_status::Model {
            id: 1,
            user_id: 7,
            envelope_id: "u7-illuminate-capture42".to_string(),
            run: 3,
            task_type: "illuminate".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 42,
            status_code: crate::task::TaskRunStatus::InProgress.as_i32(),
            attempts: 2,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let event = task_status_event_from_row(row).unwrap();
        assert_eq!(event.entity_id, 42);
        assert_eq!(event.payload.status, crate::task::TaskRunStatus::InProgress);
        assert_eq!(event.payload.run, 3);
        assert_eq!(event.payload.user_id, 7);
    }

    #[test]
    fn snapshot_row_with_unknown_status_is_skipped() {
        let row = crate::model::task_run_status::Model {
            id: 2,
            user_id: 7,
            envelope_id: "u7-illuminate-capture42".to_string(),
            run: 1,
            task_type: "illuminate".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 42,
            status_code: i32::MAX,
            attempts: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(task_status_event_from_row(row).is_none());
    }

    #[test]
    fn live_stream_forwards_only_the_authenticated_users_task_status() {
        let own = sse::ServerEvent::<sse::TaskStatusPayload>::task_status(
            Utc::now(),
            "capture",
            42,
            "illuminate",
            crate::task::TaskRunStatus::InProgress,
            1,
            1,
            7,
        );
        let other = sse::ServerEvent::<sse::TaskStatusPayload>::task_status(
            Utc::now(),
            "capture",
            43,
            "illuminate",
            crate::task::TaskRunStatus::InProgress,
            1,
            1,
            8,
        );

        assert!(task_status_for_user(sse::ReceivedServerEvent::TaskStatus(own), 7).is_some());
        assert!(task_status_for_user(sse::ReceivedServerEvent::TaskStatus(other), 7).is_none());
        assert!(
            task_status_for_user(
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
    async fn stream_emits_snapshot_then_user_wide_live_events() {
        let snapshot = task_status_event(7, 42, crate::task::TaskRunStatus::InProgress);
        let (sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(task_status_stream(
            vec![snapshot.clone()],
            receiver,
            7,
            shutdown_receiver,
        ));

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("snapshot should be immediate")
                .unwrap(),
            snapshot,
            "snapshot must be the first event"
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
            .expect("live event should follow the snapshot")
            .unwrap();
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
        let mut events = Box::pin(task_status_stream(
            Vec::new(),
            receiver,
            7,
            shutdown_receiver,
        ));

        shutdown_sender.send(true).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("shutdown should end stream promptly")
                .is_none()
        );
    }

    fn task_status_event(
        user_id: i32,
        entity_id: i32,
        status: crate::task::TaskRunStatus,
    ) -> sse::TaskStatusEvent {
        sse::ServerEvent::task_status(
            Utc::now(),
            "capture",
            entity_id,
            "illuminate",
            status,
            1,
            1,
            user_id,
        )
    }
}
