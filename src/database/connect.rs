use std::time::Duration;

use sea_orm::{ConnectionTrait, Database, DbErr, Statement};
use tracing::log::LevelFilter;

use super::config::{DbConfig, DbHandle};

pub async fn connect(dbconfig: DbConfig) -> Result<DbHandle, DbErr> {
    let mut options = sea_orm::ConnectOptions::new(dbconfig.to_url());

    // sqlx logs all queries to INFO by default, so we set to DEBUG
    options.sqlx_logging_level(LevelFilter::Debug);
    options.sqlx_slow_statements_logging_settings(LevelFilter::Warn, Duration::from_secs(1));

    let conn = Database::connect(options).await.map_err(|e| {
        tracing::error!(
            "Failed to connect to database at url: {}",
            dbconfig.to_url()
        );
        if matches!(dbconfig, DbConfig::SqliteFile { .. }) {
            tracing::warn!("When running in local dev, must run from the project root directory.");
        }
        e
    })?;

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    if let DbConfig::SqliteFile { .. } = &dbconfig {
        conn.execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA journal_mode=WAL;",
        ))
        .await?;
    }

    Ok(DbHandle::new(conn, dbconfig))
}
