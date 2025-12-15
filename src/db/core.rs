use super::config::{DbBackend, DbConfig, DbContext};

use sea_orm::{Database, DbErr};
use std::path::Path;

/// Initialize database connection and run migrations
pub async fn setup_database(config: DbConfig) -> Result<DbContext, DbErr> {
    // For SQLite files, ensure the database file and parent directories exist
    if let DbConfig::SqliteFile { path } = &config {
        // Create parent directories if they don't exist
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DbErr::Custom(format!("Failed to create database directory: {}", e))
            })?;
        }

        // Create the file if it doesn't exist
        if !Path::new(path).exists() {
            std::fs::File::create(path)
                .map_err(|e| DbErr::Custom(format!("Failed to create database file: {}", e)))?;
        }
    }

    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    let ctx = DbContext::new(conn, config);
    run_migrations(&ctx).await?;

    Ok(ctx)
}

/// Get a database connection without running migrations
pub async fn connect(config: DbConfig) -> Result<DbContext, DbErr> {
    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    Ok(DbContext::new(conn, config))
}

pub async fn run_migrations(ctx: &DbContext) -> Result<(), DbErr> {
    match ctx.backend() {
        DbBackend::Sqlite => super::sqlite::run_migrations(&ctx.conn).await,
        DbBackend::Postgres => super::postgressql::run_migrations_postgres(&ctx.conn).await,
    }
}
