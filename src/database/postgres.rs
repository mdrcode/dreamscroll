use anyhow;
use sea_orm::{self, DbErr};
use sqlx;
use tower_sessions_sqlx_store::PostgresStore;

use crate::config;

pub async fn connect(
    cfg: &config::Config,
) -> anyhow::Result<(sea_orm::DatabaseConnection, PostgresStore)> {
    let pool = create_postgres_pool(cfg).await?;
    let db_connection = connect_postgres_db(pool.clone()).await?;
    let session_store = connect_postgres_session_store(pool.clone()).await?;
    Ok((db_connection, session_store))
}

pub async fn create_postgres_pool(cfg: &config::Config) -> anyhow::Result<sqlx::postgres::PgPool> {
    let url = make_url_from_config(cfg, None, false);
    let url_redacted = make_url_from_config(cfg, None, true);

    tracing::info!(
        url_redacted = %url_redacted,
        "Connecting to Postgres connection pool..."
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5) // low for db-f1-micro's constraint
        .connect(&url)
        .await?;

    Ok(pool)
}

pub async fn connect_postgres_db(
    pool: sqlx::postgres::PgPool,
) -> Result<sea_orm::DatabaseConnection, DbErr> {
    let conn = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool);

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    tracing::info!("Successfully synchronized Postgres database schema");

    Ok(conn)
}

pub async fn connect_postgres_session_store(
    pool: sqlx::PgPool,
) -> anyhow::Result<tower_sessions_sqlx_store::PostgresStore> {
    let store = tower_sessions_sqlx_store::PostgresStore::new(pool);
    store.migrate().await?;
    Ok(store)
}

/// Build a Postgres connection URL from the app config.
///
/// `schema`, when set, pins the connection's `search_path` to that schema via
/// the `options` parameter. The test harness uses this to isolate each test in
/// its own schema.
///
/// `redacted` replaces the password with `<REDACTED>` for logging.
pub fn make_url_from_config(cfg: &config::Config, schema: Option<&str>, redacted: bool) -> String {
    make_url(
        &cfg.postgres_user,
        &cfg.postgres_password,
        &cfg.postgres_host_port,
        &cfg.postgres_db,
        cfg.postgres_connection_params.as_deref(),
        schema,
        redacted,
    )
}

/// Build a Postgres connection URL from its parts.
///
/// Prefer [`make_url_from_config`] when you have a [`config::Config`].
fn make_url(
    user: &str,
    password: &str,
    host_port: &str,
    db: &str,
    connection_params: Option<&str>,
    schema: Option<&str>,
    redacted: bool,
) -> String {
    let mut query_params: Vec<String> = Vec::new();

    // e.g. "sslmode=require"
    if let Some(connection_params) = connection_params {
        query_params.push(connection_params.to_string());
    }

    if let Some(schema) = schema {
        query_params.push(format!("options=-csearch_path%3D{schema}"));
    }

    let query = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };

    if redacted {
        format!(
            "postgres://{}:<REDACTED>@{}/{}{}",
            user, host_port, db, query
        )
    } else {
        format!(
            "postgres://{}:{}@{}/{}{}",
            user, password, host_port, db, query
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "alice";
    const PASSWORD: &str = "secret";
    const HOST_PORT: &str = "db.internal:5432";
    const DB: &str = "dreamscroll";

    #[test]
    fn make_url_without_connection_params() {
        let url = make_url(USER, PASSWORD, HOST_PORT, DB, None, None, false);

        assert_eq!(url, "postgres://alice:secret@db.internal:5432/dreamscroll");
    }

    #[test]
    fn make_url_with_connection_params() {
        let url = make_url(
            USER,
            PASSWORD,
            HOST_PORT,
            DB,
            Some("sslmode=require&application_name=dreamscroll"),
            None,
            false,
        );

        assert_eq!(
            url,
            "postgres://alice:secret@db.internal:5432/dreamscroll?sslmode=require&application_name=dreamscroll"
        );
    }

    #[test]
    fn make_url_with_schema_override() {
        let url = make_url(
            USER,
            PASSWORD,
            HOST_PORT,
            DB,
            None,
            Some("test_abc123"),
            false,
        );

        assert_eq!(
            url,
            "postgres://alice:secret@db.internal:5432/dreamscroll?options=-csearch_path%3Dtest_abc123"
        );
    }

    #[test]
    fn make_url_combines_connection_params_and_schema() {
        let url = make_url(
            USER,
            PASSWORD,
            HOST_PORT,
            DB,
            Some("sslmode=require"),
            Some("test_abc123"),
            false,
        );

        assert_eq!(
            url,
            "postgres://alice:secret@db.internal:5432/dreamscroll?sslmode=require&options=-csearch_path%3Dtest_abc123"
        );
    }

    #[test]
    fn make_url_redacts_password() {
        let url = make_url(
            USER,
            PASSWORD,
            HOST_PORT,
            DB,
            Some("sslmode=require"),
            None,
            true,
        );

        assert_eq!(
            url,
            "postgres://alice:<REDACTED>@db.internal:5432/dreamscroll?sslmode=require"
        );
        assert!(!url.contains("secret"));
    }
}
