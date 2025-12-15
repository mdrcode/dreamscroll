use super::{
    config::{DbBackend, DbConfig, DbHandle},
    postgres, sqlite,
};

use sea_orm::{Database, DbErr};
use std::path::Path;

pub async fn connect(config: DbConfig) -> Result<DbHandle, DbErr> {
    // SqliteFile will not create parent directories automatically, so must do manually
    if let DbConfig::SqliteFile { path } = &config {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DbErr::Custom(format!("Failed to create database parent dirs: {}", e))
            })?;
        }
    }

    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    Ok(DbHandle::new(conn, config))
}

pub async fn run_migrations(handle: &DbHandle) -> Result<(), DbErr> {
    match handle.backend {
        DbBackend::Sqlite => sqlite::run_migrations(&handle.conn).await,
        DbBackend::Postgres => postgres::run_migrations(&handle.conn).await,
    }
}
