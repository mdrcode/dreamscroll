use std::time::Duration;

use sea_orm::{ConnectionTrait, Database, DbErr, Statement};
use tracing::log::LevelFilter;

mod config;
pub use config::{DbBackend, DbConfig, DbHandle};

pub async fn connect(config: DbConfig) -> Result<DbHandle, DbErr> {
    let mut options = sea_orm::ConnectOptions::new(config.to_url());
    options.sqlx_logging_level(LevelFilter::Debug);
    options.sqlx_slow_statements_logging_settings(LevelFilter::Warn, Duration::from_secs(1));

    let conn = Database::connect(options).await;

    if let Err(e) = conn {
        tracing::error!("Failed to connect to database at '{}'", config.to_url());
        if let DbConfig::SqliteFile { .. } = &config {
            tracing::warn!("When running in local dev, must run from the project root directory.");
        }

        return Err(e);
    }

    let conn = conn?;

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    if let DbConfig::SqliteFile { .. } = &config {
        conn.execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA journal_mode=WAL;",
        ))
        .await?;
    }

    Ok(DbHandle::new(conn, config))
}
