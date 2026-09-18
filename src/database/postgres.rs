use anyhow;
use sea_orm::{self, DbErr};
use sqlx08;
use tower_sessions_sqlx_store::PostgresStore;

use crate::config;

pub async fn connect(
    cfg: &config::Config,
) -> anyhow::Result<(sea_orm::DatabaseConnection, PostgresStore)> {
    // TODO: Currently we must support TWO CONNECTION POOLS to account
    // for sqlx version mismatch between sea-orm and tower_sessions_sqlx_store.
    // Once tower_sessions_sqlx_store is updated to use sqlx 0.9, we can remove
    // the second pool. This means currently we are doubling up on connection
    // budget.
    let db_connection = connect_postgres_db(cfg).await?; // pool #1 internally
    let sqlx08_pool = create_session_sqlx08_pool(cfg).await?; // pool #2
    let session_store = connect_postgres_session_store(sqlx08_pool).await?;
    Ok((db_connection, session_store))
}

pub async fn connect_postgres_db(
    cfg: &config::Config,
) -> Result<sea_orm::DatabaseConnection, DbErr> {
    let url = make_url_from_config(cfg, None, false);
    let mut options = sea_orm::ConnectOptions::new(url);
    options.max_connections(5).min_connections(0); // TODO make configurable

    let conn = sea_orm::Database::connect(options).await?;

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    tracing::info!("Successfully synchronized Postgres database schema");

    Ok(conn)
}

async fn create_session_sqlx08_pool(cfg: &config::Config) -> anyhow::Result<sqlx08::PgPool> {
    let url = make_url_from_config(cfg, None, false);
    Ok(sqlx08::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?)
}

pub async fn connect_postgres_session_store(
    pool: sqlx08::PgPool,
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
