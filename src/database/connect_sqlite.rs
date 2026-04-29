use anyhow;
use sea_orm::{self, ConnectionTrait, DbErr, Statement};
use sqlx;

use crate::auth;

pub async fn create_sqlite_pool(path: &str) -> anyhow::Result<sqlx::sqlite::SqlitePool> {
    tracing::info!("Connecting to SQLite database at path: {}", path);

    // create dirs if they don't exist
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(20)
        .connect(path)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(pool)
}

pub async fn connect_sqlite_db(
    pool: sqlx::sqlite::SqlitePool,
) -> Result<sea_orm::DatabaseConnection, DbErr> {
    let conn = sea_orm::SqlxSqliteConnector::from_sqlx_sqlite_pool(pool.clone());

    // Ensure UTF-8 encoding (must be set before table creation for new databases)
    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "PRAGMA encoding = 'UTF-8';",
    ))
    .await?;

    conn.execute_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "PRAGMA journal_mode=WAL;",
    ))
    .await?;

    let result = conn
        .get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await;

    if let Err(e) = result {
        tracing::error!("Error syncing schema registry: {:?}", e);
        return Err(e);
    }

    Ok(conn)
}

pub async fn connect_sqlite_session_store(
    pool: sqlx::SqlitePool,
) -> anyhow::Result<auth::SessionStoreWrapper> {
    let store = tower_sessions_sqlx_store::SqliteStore::new(pool);
    store.migrate().await?;
    Ok(auth::SessionStoreWrapper::Sqlite(store))
}
