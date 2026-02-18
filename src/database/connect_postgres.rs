use anyhow;
use sea_orm::{self, DbErr};
use sqlx;

use crate::auth;

pub async fn create_postgres_pool(url: &str) -> anyhow::Result<sqlx::postgres::PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(url)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!("Successfully created Postgres connection pool");
    Ok(pool)
}

pub async fn connect_postgres_db(
    pool: sqlx::postgres::PgPool,
) -> Result<sea_orm::DatabaseConnection, DbErr> {
    let conn = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool.clone());

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    tracing::info!("Successfully synchronized Postgres database schema");

    Ok(conn)
}

pub async fn connect_postgres_session_store(
    pool: sqlx::PgPool,
) -> anyhow::Result<auth::SessionStoreWrapper> {
    let store = tower_sessions_sqlx_store::PostgresStore::new(pool);
    store.migrate().await?;
    Ok(auth::SessionStoreWrapper::Postgres(store))
}
