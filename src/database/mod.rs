mod config;
mod postgres;
mod sqlite;

pub use config::{DbBackend, DbConfig, DbHandle};

use sea_orm::{ConnectionTrait, Database, DbErr, Statement};

pub async fn connect(config: DbConfig) -> Result<DbHandle, DbErr> {
    // SqliteFile will not create parent directories automatically, must do manually
    if let DbConfig::SqliteFile { path } = &config {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DbErr::Custom(format!("Failed to create database parent dirs: {}", e))
            })?;
        }
    }

    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    if let DbConfig::SqliteFile { .. } = &config {
        conn.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA journal_mode=WAL;",
        ))
        .await?;
    }

    Ok(DbHandle::new(conn, config))
}

pub async fn run_migrations(handle: &DbHandle) -> Result<(), DbErr> {
    match handle.backend {
        DbBackend::Sqlite => sqlite::run_migrations(&handle.conn).await,
        DbBackend::Postgres => postgres::run_migrations(&handle.conn).await,
    }
}
