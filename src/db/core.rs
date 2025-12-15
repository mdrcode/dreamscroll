use super::config::{DatabaseBackend, DatabaseConfig, DbContext};

use sea_orm::{Database, DbErr};

/// Initialize database connection and run migrations
pub async fn setup_database(config: DatabaseConfig) -> Result<DbContext, DbErr> {
    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    let ctx = DbContext::new(conn, config);
    run_migrations(&ctx).await?;

    Ok(ctx)
}

/// Get a database connection without running migrations
pub async fn connect(config: DatabaseConfig) -> Result<DbContext, DbErr> {
    let url = config.to_url();
    let conn = Database::connect(&url).await?;

    Ok(DbContext::new(conn, config))
}

pub async fn run_migrations(ctx: &DbContext) -> Result<(), DbErr> {
    match ctx.backend() {
        DatabaseBackend::Sqlite => super::sqlite::run_migrations(&ctx.conn).await,
        DatabaseBackend::Postgres => super::postgressql::run_migrations_postgres(&ctx.conn).await,
    }
}
