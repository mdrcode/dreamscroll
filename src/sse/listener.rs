use anyhow::{Context, bail};
use serde_json::Value;
use sqlx::postgres::PgListener;

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
/// the listener needs to remain connected while waiting for notifications.
pub struct ServerEventListener {
    listener: PgListener,
}

impl ServerEventListener {
    /// Open a dedicated connection and subscribe to the server-event channel.
    ///
    /// The connection is owned, long-lived, and held for the listener's lifetime.
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

fn decode_server_event(payload: &str) -> anyhow::Result<ReceivedServerEvent> {
    let value: Value = serde_json::from_str(payload).context("parse server-event JSON")?;
    let event_type = value
        .get("event_type")
        .and_then(Value::as_str)
        .context("server event has no string event_type")?;

    match event_type {
        "task_status" => Ok(ReceivedServerEvent::TaskStatus(
            serde_json::from_value(value).context("decode task-status event")?,
        )),
        "availability" => Ok(ReceivedServerEvent::Availability(
            serde_json::from_value(value).context("decode availability event")?,
        )),
        other => bail!("unsupported server event type: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_task_status_event() {
        let payload = r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":{"name":"complete_success","discriminant":4},"run":3}}"#;
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
}
