use super::{ServerEvent, TaskStatusEvent};
use anyhow::Context;
use sea_orm::DatabaseConnection;
use sqlx::PgPool;

#[async_trait::async_trait]
pub trait ServerEventNotifier: Send + Sync {
    async fn notify_task_status(&self, event: &TaskStatusEvent) -> anyhow::Result<()>;
}

/// PostgreSQL channel used for best-effort server event notifications.
pub const SERVER_EVENT_CHANNEL: &str = "server_event_channel";

/// Publishes task-status events through PostgreSQL `NOTIFY`.
#[derive(Clone)]
pub struct PostgresNotifier {
    pool: PgPool,
}

impl PostgresNotifier {
    pub fn new(db: &DatabaseConnection) -> Self {
        Self {
            pool: db.get_postgres_connection_pool().clone(),
        }
    }

    pub async fn notify<E: serde::Serialize>(&self, event: &ServerEvent<E>) -> anyhow::Result<()> {
        let payload = serde_json::to_string(event).context("serialize server event")?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(SERVER_EVENT_CHANNEL)
            .bind(payload)
            .execute(&self.pool)
            .await
            .context("publish server event notification")?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ServerEventNotifier for PostgresNotifier {
    /// Publish to listeners on this database. PostgreSQL delivers notifications
    /// on transaction commit; they are best effort and not retained.
    async fn notify_task_status(&self, event: &TaskStatusEvent) -> anyhow::Result<()> {
        self.notify(event).await
    }
}
