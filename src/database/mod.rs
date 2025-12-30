mod config;

pub use config::{DbBackend, DbConfig, DbHandle};

use sea_orm::{ConnectionTrait, Database, DbErr, Statement};

pub async fn connect(config: DbConfig) -> Result<DbHandle, DbErr> {
    // SqliteFile will not create parent dirs automatically, must do manually
    if let DbConfig::SqliteFile { path } = &config {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DbErr::Custom(format!("Failed to create database parent dirs: {}", e))
            })?;
        }
    }

    let url = config.to_url();
    let conn = Database::connect(&url).await?;
    conn.get_schema_registry("dreamspot::model::*")
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
