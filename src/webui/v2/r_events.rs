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

const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(4 * 60);

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
    let live_events = state.server_events.subscribe();
    let capture_ids = parse_capture_ids(params.capture_ids.as_deref())?;
    let catchup_events = dedupe_catchup(
        state
            .task_master
            .query_latest_status_for_entities(user_id, "capture", &capture_ids)
            .await?
            .into_iter()
            .filter_map(|row| sse::TaskStatusEvent::from_row(&row, None)),
    );

    let deadline = tokio::time::Instant::now() + MAX_STREAM_LIFETIME;
    let event_stream = stream_task_status(
        catchup_events,
        live_events,
        user_id,
        state.shutdown.clone(),
        deadline,
    )
    .map(serialize_task_event);

    Ok(Sse::new(event_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(20))
            .text("keep-alive"),
    ))
}

fn stream_task_status(
    catchup: Vec<sse::TaskStatusEvent>,
    receiver: broadcast::Receiver<sse::ReceivedServerEvent>,
    user_id: i32,
    shutdown: tokio::sync::watch::Receiver<bool>,
    deadline: tokio::time::Instant,
) -> impl futures_util::Stream<Item = sse::TaskStatusEvent> {
    let live_events = stream::unfold(
        (receiver, user_id, shutdown, deadline),
        |(mut receiver, user_id, mut shutdown, deadline)| async move {
            loop {
                if *shutdown.borrow() {
                    return None;
                }

                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => return None,
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return None;
                        }
                    }
                    received = receiver.recv() => {
                        match received {
                            Ok(received) => {
                                if let Some(update) = filter_for_user(received, user_id) {
                                    return Some((
                                        update,
                                        (receiver, user_id, shutdown, deadline),
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
    stream::iter(catchup).chain(live_events)
}

/// Keep only the newest catch-up status per entity to avoid duplicate partial refreshes.
fn dedupe_catchup(
    events: impl IntoIterator<Item = sse::TaskStatusEvent>,
) -> Vec<sse::TaskStatusEvent> {
    let mut latest_by_entity: std::collections::HashMap<(String, i32), sse::TaskStatusEvent> =
        std::collections::HashMap::new();
    for event in events {
        let key = (event.entity_type.clone(), event.entity_id);
        match latest_by_entity.get(&key) {
            Some(existing) if existing.timestamp >= event.timestamp => {}
            _ => {
                latest_by_entity.insert(key, event);
            }
        }
    }
    latest_by_entity.into_values().collect()
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
    }

    Ok(ids)
}

fn serialize_task_event(update: sse::TaskStatusEvent) -> Result<Event, Infallible> {
    Ok(Event::default()
        .event("task-status")
        .data(task_event_json(&update)))
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
                attempts: 1,
                run: 2,
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
            processing_started_at: None,
            last_error_duration_ms: None,
            success_duration_ms: None,
            updated_at: Utc::now(),
        };

        let event = sse::TaskStatusEvent::from_row(&row, None).unwrap();
        assert_eq!(event.entity_id, 42);
        assert_eq!(event.payload.status, crate::task::TaskRunStatus::InProgress);
        assert_eq!(event.payload.run, 3);
        assert_eq!(event.user_id, Some(7));
    }

    #[test]
    fn catchup_row_with_unknown_status_is_skipped() {
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
            processing_started_at: None,
            last_error_duration_ms: None,
            success_duration_ms: None,
            updated_at: Utc::now(),
        };

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
        let mut events = Box::pin(stream_task_status(
            vec![catchup.clone()],
            receiver,
            7,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
        ));

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("catch-up should be immediate")
                .unwrap(),
            catchup,
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
        assert_eq!(live.entity_id, 999);
        assert_eq!(
            live.payload.status,
            crate::task::TaskRunStatus::CompleteSuccess
        );
    }

    #[test]
    fn catchup_keeps_only_latest_event_per_entity() {
        let mut earlier = task_status_event(7, 42, crate::task::TaskRunStatus::Queued);
        earlier.timestamp = Utc::now() - chrono::Duration::seconds(1);
        let later = task_status_event(7, 42, crate::task::TaskRunStatus::CompleteSuccess);
        let other_entity = task_status_event(7, 43, crate::task::TaskRunStatus::InProgress);

        let deduplicated = dedupe_catchup(vec![earlier, later.clone(), other_entity]);

        assert_eq!(deduplicated.len(), 2);
        assert!(deduplicated.iter().any(|event| event == &later));
        assert!(deduplicated.iter().any(|event| event.entity_id == 43));
    }

    #[tokio::test]
    async fn shutdown_ends_a_stream_even_before_first_event() {
        let (_sender, receiver) = broadcast::channel(8);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let mut events = Box::pin(stream_task_status(
            Vec::new(),
            receiver,
            7,
            shutdown_receiver,
            tokio::time::Instant::now() + Duration::from_secs(10),
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
    async fn server_deadline_ends_a_live_stream() {
        let (_sender, receiver) = broadcast::channel(8);
        let (_shutdown_sender, shutdown_receiver) = watch::channel(false);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let mut events = Box::pin(stream_task_status(
            Vec::new(),
            receiver,
            7,
            shutdown_receiver,
            deadline,
        ));

        assert!(
            tokio::time::timeout(Duration::from_secs(1), events.next())
                .await
                .expect("bounded server stream should finish")
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
                    run: 1,
                    processing_started_at: None,
                    estimated_duration_ms_p50: None,
                },
            )
        }
    }
}
