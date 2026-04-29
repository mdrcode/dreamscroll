use anyhow;
use sea_orm::{self, DbErr};
use sqlx;

use crate::auth;

use super::*;

pub async fn create_postgres_pool(
    config: &PostgresConfig,
) -> anyhow::Result<sqlx::postgres::PgPool> {
    let url = make_url(config, false);
    let url_redacted = make_url(config, true);

    tracing::info!(
        url_redacted = %url_redacted,
        "Connecting to Postgres connection pool..."
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
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
) -> anyhow::Result<auth::SessionStoreWrapper> {
    let store = tower_sessions_sqlx_store::PostgresStore::new(pool);
    store.migrate().await?;
    Ok(auth::SessionStoreWrapper::Postgres(store))
}

fn make_url(config: &PostgresConfig, redacted: bool) -> String {
    // e.g. "sslmode=require"
    let params = config
        .connection_params
        .as_deref()
        .map(|params| format!("?{}", params))
        .unwrap_or_default();

    if redacted {
        format!(
            "postgres://{}:<REDACTED>@{}/{}{}",
            &config.user, &config.host_port, &config.db, params
        )
    } else {
        format!(
            "postgres://{}:{}@{}/{}{}",
            &config.user, &config.password, &config.host_port, &config.db, params
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(connection_params: Option<&str>) -> PostgresConfig {
        PostgresConfig {
            user: "alice".to_string(),
            password: "secret".to_string(),
            host_port: "db.internal:5432".to_string(),
            db: "dreamscroll".to_string(),
            connection_params: connection_params.map(str::to_string),
        }
    }

    #[test]
    fn make_url_without_connection_params() {
        let config = test_config(None);

        let url = make_url(&config, false);

        assert_eq!(url, "postgres://alice:secret@db.internal:5432/dreamscroll");
    }

    #[test]
    fn make_url_with_connection_params() {
        let config = test_config(Some("sslmode=require&application_name=dreamscroll"));

        let url = make_url(&config, false);

        assert_eq!(
            url,
            "postgres://alice:secret@db.internal:5432/dreamscroll?sslmode=require&application_name=dreamscroll"
        );
    }

    #[test]
    fn make_url_redacts_password() {
        let config = test_config(Some("sslmode=require"));

        let url = make_url(&config, true);

        assert_eq!(
            url,
            "postgres://alice:<REDACTED>@db.internal:5432/dreamscroll?sslmode=require"
        );
        assert!(!url.contains("secret"));
    }
}
