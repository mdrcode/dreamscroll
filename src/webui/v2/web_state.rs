use sea_orm::ConnectionTrait;
use tera::Context;

use crate::{api, sse, task};

pub struct WebState {
    pub user_api: api::UserApiClient,
    pub task_master: std::sync::Arc<task::TaskMaster>,
    pub server_events: tokio::sync::broadcast::Sender<sse::ReceivedServerEvent>,
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    pub tera: tera::Tera,
    pub static_asset_version: String,
    pub max_upload_bytes: usize,
}

impl WebState {
    pub fn template_context(&self) -> Context {
        let mut context = Context::new();
        context.insert("static_asset_version", &self.static_asset_version);
        context
    }

    pub async fn current_db_timestamp(
        &self,
    ) -> Result<chrono::DateTime<chrono::Utc>, sea_orm::DbErr> {
        use sea_orm::{DatabaseBackend, Statement};

        let row = self
            .user_api
            .db
            .conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT CURRENT_TIMESTAMP AS snapshot_at",
            ))
            .await?
            .expect("SELECT CURRENT_TIMESTAMP always returns one row");
        row.try_get_by_index(0)
    }
}
