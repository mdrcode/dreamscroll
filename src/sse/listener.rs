use anyhow::Context;
use serde_json::Value;
use sqlx::postgres::PgListener;
use tokio::sync::{broadcast, watch};

use super::*;

/// Owns a dedicated, long-lived PostgreSQL connection required by `LISTEN`.
///
/// This must not use a connection borrowed from the application's `PgPool`:
/// the listener needs to remain connected forever while waiting for
/// notifications.
pub struct ServerEventListener {
    listener: PgListener,
}

impl ServerEventListener {
    /// Open a dedicated connection and subscribe to the server-event channel.
    ///
    /// The connection is exclusively owned and held for the listener's lifetime.
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let mut listener = PgListener::connect(database_url)
            .await
            .context("connect server-event listener")?;

        listener
            .listen(notifier::SERVER_EVENT_CHANNEL)
            .await
            .context("listen for server events")?;
        Ok(Self { listener })
    }

    /// Wait for the next notification and decode it.
    ///
    /// `Ok(None)` means the payload was skipped (unknown kind, wrong schema
    /// version, malformed); only a transport failure is returned, because that is
    /// what ends the fan-out. SQLx 0.9 reconnects after intermittent connection
    /// loss on its own.
    pub async fn recv(&mut self) -> anyhow::Result<Option<TaskStatusUpdate>> {
        let notification = self
            .listener
            .recv()
            .await
            .context("receive server-event notification")?;
        Ok(decode_server_event(notification.payload()))
    }
}

/// Start the dedicated Postgres listener and expose its decoded notifications
/// to SSE connections on this instance. Lagged receivers may miss hints; that
/// is acceptable for this informational stream.
///
/// PgListener::recv() handles transient Postgres connection loss itself. If
/// recv returns an error, this task logs and exits; the SSE client can reconnect
/// but cannot revive this fan-out task. See `plan/archive/sse.md` §6 for the distinction.
pub fn spawn_local_fanout(
    mut listener: ServerEventListener,
    mut shutdown: watch::Receiver<bool>,
) -> broadcast::Sender<TaskStatusUpdate> {
    let (sender, _) = broadcast::channel(128); // TODO where did 128 come from?
    let task_sender = sender.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        tracing::info!("Server-event listener shutting down");
                        break;
                    }
                }
                result = listener.recv() => {
                    match result {
                        Ok(Some(event)) => { let _ = task_sender.send(event); }
                        // Skipped payloads are already logged; keep listening.
                        Ok(None) => {}
                        // No retry: the sender outlives this task, so a dead
                        // fan-out looks like an idle stream until restart.
                        Err(error) => {
                            tracing::warn!(error = ?error, "Server-event listener stopped");
                            break;
                        }
                    }
                }
            }
        }
    });

    sender
}

/// Decode one notification, skipping anything this build does not understand.
///
/// A bad payload must never stop the fan-out: skipping keeps live updates working,
/// and a `schema_version` bump during a rolling deploy would otherwise kill the
/// listener on every instance still running the previous build.
fn decode_server_event(payload: &str) -> Option<TaskStatusUpdate> {
    let value: Value = match serde_json::from_str(payload) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = ?error, "Skipping undecodable server-event JSON");
            return None;
        }
    };

    let schema_version = value.get("schema_version").and_then(Value::as_u64);
    if schema_version != Some(u64::from(CURRENT_SCHEMA_VERSION)) {
        tracing::warn!(
            schema_version = ?schema_version,
            expected = CURRENT_SCHEMA_VERSION,
            "Skipping server event with an unsupported schema_version"
        );
        return None;
    }

    let Some(kind_value) = value.get("event_type") else {
        tracing::warn!("Skipping server event with no event_type");
        return None;
    };
    let kind: ServerEventKind = match serde_json::from_value(kind_value.clone()) {
        Ok(kind) => kind,
        Err(error) => {
            tracing::warn!(
                error = ?error,
                event_type = ?kind_value,
                "Skipping server event with an unknown event_type"
            );
            return None;
        }
    };

    // Matching the enum turns a new kind into a compile error here rather than a
    // notification that is silently dropped.
    match kind {
        ServerEventKind::TaskStatus => decode_task_status_update(value),
    }
}

/// Decode the `task_status` body of an envelope that already passed the version
/// and kind gates.
fn decode_task_status_update(value: Value) -> Option<TaskStatusUpdate> {
    let event: TaskStatusUpdate = match serde_json::from_value(value) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(error = ?error, "Skipping malformed task-status event");
            return None;
        }
    };
    if !event.entity_owner_id.is_some_and(|owner_id| owner_id > 0) {
        tracing::warn!(
            run_id = %event.payload.run_id,
            "Skipping task-status event without an entity owner"
        );
        return None;
    }

    Some(event)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;

    const TASK_STATUS: &str = r#"{"schema_version":4,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"entity_owner_id":7,"payload":{"task_type":"illuminate","status":{"name":"complete_success","discriminant":4},"logical_id":"u7-illuminate-capture42","run_id":"b91a7c4f-7e8a-4bf8-9a76-c81e258ec113","run_number":3,"attempts":1}}"#;

    #[test]
    fn decodes_task_status_event() {
        let event = decode_server_event(TASK_STATUS).expect("task-status payload should decode");
        assert_eq!(event.payload.run_id, "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113");
        assert_eq!(event.entity_owner_id, Some(7));
    }

    #[test]
    fn skips_unsupported_event_type() {
        let availability = r#"{"schema_version":4,"event_type":"availability","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"operation":"deleted"}}"#;
        assert!(decode_server_event(availability).is_none());
    }

    #[test]
    fn skips_missing_and_unsupported_schema_versions() {
        let missing = r#"{"event_type":"task_status"}"#;
        let unsupported = r#"{"schema_version":5,"event_type":"task_status"}"#;
        let negative = r#"{"schema_version":-1,"event_type":"task_status"}"#;

        assert!(decode_server_event(missing).is_none());
        assert!(decode_server_event(unsupported).is_none());
        assert!(decode_server_event(negative).is_none());
    }

    #[test]
    fn skips_task_status_without_an_entity_owner() {
        let no_owner = TASK_STATUS.replace(r#""entity_owner_id":7,"#, "");
        assert!(decode_server_event(&no_owner).is_none());
    }

    #[test]
    fn skips_malformed_task_status_payload() {
        let malformed = TASK_STATUS.replace(
            r#""status":{"name":"complete_success","discriminant":4}"#,
            r#""status":"not-an-object""#,
        );
        assert!(decode_server_event(&malformed).is_none());
    }

    #[tokio::test]
    async fn a_skipped_notification_does_not_stop_the_fanout() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let config = crate::test_support::test_config::load()
            .expect("test configuration should load when test DB is available");
        let database_url = crate::database::make_url_from_config(&config, None, false);
        let listener = ServerEventListener::connect(&database_url)
            .await
            .expect("listener should connect and LISTEN");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let events = spawn_local_fanout(listener, shutdown_rx);
        let mut receiver = events.subscribe();
        let db = db.handle();
        let notifier = super::super::PostgresNotifier::new(&db.conn);

        let event_id = (uuid::Uuid::new_v4().as_u128() as u32) as i32;
        let task_status = super::super::ServerEventEnvelope {
            entity_owner_id: Some(741_258),
            ..TaskStatusUpdate::new(
                ServerEventKind::TaskStatus,
                Utc::now(),
                "capture",
                event_id,
                super::super::TaskStatusPayload {
                    task_type: "illuminate".to_string(),
                    status: crate::task::TaskRunStatus::InProgress,
                    logical_id: format!("u741258-illuminate-capture{event_id}"),
                    run_id: "6f6844ec-d1ac-4f51-b4ae-57e92c646b8d".to_string(),
                    run_number: 3,
                    attempts: 2,
                    processing_started_at: None,
                    estimated_duration_ms_p50: None,
                },
            )
        };
        // A version this build does not understand must be skipped, not fatal.
        let from_a_newer_build = super::super::ServerEventEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..task_status.clone()
        };
        notifier
            .notify(&from_a_newer_build)
            .await
            .expect("unsupported notification should publish");

        notifier
            .notify(&task_status)
            .await
            .expect("task status notification should publish");

        let received = receive_matching_entity(&mut receiver, event_id).await;
        assert_eq!(received, task_status);

        shutdown_tx.send(true).expect("fanout is still subscribed");
    }

    async fn receive_matching_entity(
        receiver: &mut broadcast::Receiver<TaskStatusUpdate>,
        entity_id: i32,
    ) -> TaskStatusUpdate {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = receiver.recv().await.expect("fanout should remain open");
                if event.entity_id == entity_id {
                    return event;
                }
            }
        })
        .await
        .expect("notification should reach the local fanout promptly")
    }
}
