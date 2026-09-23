use anyhow::Context;
use sqlx::PgPool;

use super::{ServerEvent, event::ServerEventTypes};

/// PostgreSQL channel used for best-effort server event notifications.
pub const SERVER_EVENT_CHANNEL: &str = "server_event_channel";

/// Publishes typed server events through PostgreSQL `NOTIFY`.
#[derive(Clone)]
pub struct ServerEventNotifier {
    pool: PgPool,
}

impl ServerEventNotifier {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Publish an event to listeners on this database.
    ///
    /// PostgreSQL delivers this notification on transaction commit. Notifications
    /// are best effort and are not retained for disconnected listeners.
    pub async fn notify<E: serde::Serialize>(
        &self,
        event: &ServerEvent<E>,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_string(event).context("serialize server event")?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(SERVER_EVENT_CHANNEL)
            .bind(payload)
            .execute(&self.pool)
            .await
            .context("publish server event notification")?;
        Ok(())
    }

    /// Publish a typed task-status event.
    pub async fn notify_task_status(
        &self,
        event: &super::TaskStatusEvent,
    ) -> anyhow::Result<()> {
        if event.event_type != ServerEventTypes::TaskStatus {
            anyhow::bail!("expected a task-status server event");
        }
        self.notify(event).await
    }
}
