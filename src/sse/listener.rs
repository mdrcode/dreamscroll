use anyhow::{Context, bail};
use serde_json::Value;
use sqlx::postgres::PgListener;
use tokio::sync::{broadcast, watch};

use super::{AvailabilityEvent, TaskStatusEvent, notifier::SERVER_EVENT_CHANNEL};

/// A decoded notification received from the shared server-event channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceivedServerEvent {
    TaskStatus(TaskStatusEvent),
    Availability(AvailabilityEvent),
}

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
            .listen(SERVER_EVENT_CHANNEL)
            .await
            .context("listen for server events")?;
        Ok(Self { listener })
    }

    /// Wait for and decode the next best-effort server event.
    pub async fn recv(&mut self) -> anyhow::Result<ReceivedServerEvent> {
        let notification = self
            .listener
            .recv()
            .await
            .context("receive server-event notification")?;
        decode_server_event(notification.payload())
    }
}

/// Start the dedicated Postgres listener and expose its decoded notifications
/// to SSE connections on this instance. Lagged receivers may miss hints; that
/// is acceptable for this informational stream.
pub fn spawn_local_fanout(
    mut listener: ServerEventListener,
    mut shutdown: watch::Receiver<bool>,
) -> broadcast::Sender<ReceivedServerEvent> {
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
                        Ok(event) => { let _ = task_sender.send(event); }
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

fn decode_server_event(payload: &str) -> anyhow::Result<ReceivedServerEvent> {
    let value: Value = serde_json::from_str(payload).context("parse server-event JSON")?;
    let event_type = value
        .get("event_type")
        .and_then(Value::as_str)
        .context("server event has no string event_type")?;

    match event_type {
        "task_status" => {
            let event: TaskStatusEvent =
                serde_json::from_value(value).context("decode task-status event")?;
            if event.payload.user_id == 0 {
                bail!("PostgreSQL task-status notification has no routing user_id");
            }
            Ok(ReceivedServerEvent::TaskStatus(event))
        }
        "availability" => Ok(ReceivedServerEvent::Availability(
            serde_json::from_value(value).context("decode availability event")?,
        )),
        other => bail!("unsupported server event type: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;

    #[test]
    fn decodes_task_status_event() {
        let payload = r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":{"name":"complete_success","discriminant":4},"attempts":1,"run":3,"user_id":7}}"#;
        assert!(matches!(
            decode_server_event(payload).unwrap(),
            ReceivedServerEvent::TaskStatus(_)
        ));
    }

    #[test]
    fn decodes_availability_event() {
        let payload = r#"{"schema_version":1,"event_type":"availability","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"operation":"deleted"}}"#;
        assert!(matches!(
            decode_server_event(payload).unwrap(),
            ReceivedServerEvent::Availability(_)
        ));
    }

    #[test]
    fn rejects_unknown_event_type() {
        let payload = r#"{"event_type":"unknown"}"#;
        assert!(decode_server_event(payload).is_err());
    }

    #[test]
    fn rejects_task_status_notification_without_owner_id() {
        let payload = r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":{"name":"complete_success","discriminant":4},"attempts":1,"run":3}}"#;
        let error = decode_server_event(payload).unwrap_err();
        assert!(error.to_string().contains("routing user_id"));
    }

    #[test]
    fn rejects_malformed_task_status_payload() {
        let payload = r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":"not-an-object"}}"#;
        assert!(decode_server_event(payload).is_err());
    }

    #[tokio::test]
    async fn postgres_notification_reaches_local_fanout() {
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
        let notifier = super::super::ServerEventNotifier::new(
            db.handle().conn.get_postgres_connection_pool().clone(),
        );

        let event_id = (uuid::Uuid::new_v4().as_u128() as u32) as i32;
        let task_status = super::super::TaskStatusEvent::task_status(
            Utc::now(),
            "capture",
            event_id,
            "illuminate",
            crate::task::TaskRunStatus::InProgress,
            2,
            3,
            741_258,
        );
        notifier
            .notify(&task_status)
            .await
            .expect("task status notification should publish");

        let received_task = receive_matching_entity(&mut receiver, event_id).await;
        assert_eq!(received_task, ReceivedServerEvent::TaskStatus(task_status));

        let availability_id = event_id.wrapping_add(1);
        let availability = super::super::AvailabilityEvent::new(
            super::super::ServerEventTypes::Availability,
            Utc::now(),
            "capture",
            availability_id,
            super::super::AvailabilityPayload {
                operation: super::super::AvailabilityState::Deleted,
            },
        );
        notifier
            .notify(&availability)
            .await
            .expect("availability notification should publish");

        let received_availability = receive_matching_entity(&mut receiver, availability_id).await;
        assert_eq!(
            received_availability,
            ReceivedServerEvent::Availability(availability)
        );

        shutdown_tx.send(true).expect("fanout is still subscribed");
    }

    async fn receive_matching_entity(
        receiver: &mut broadcast::Receiver<ReceivedServerEvent>,
        entity_id: i32,
    ) -> ReceivedServerEvent {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = receiver.recv().await.expect("fanout should remain open");
                let received_id = match &event {
                    ReceivedServerEvent::TaskStatus(event) => event.entity_id,
                    ReceivedServerEvent::Availability(event) => event.entity_id,
                };
                if received_id == entity_id {
                    return event;
                }
            }
        })
        .await
        .expect("notification should reach the local fanout promptly")
    }
}
