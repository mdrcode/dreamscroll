//! Isolated-database harness for DB tests.
//!
//! Tests that need a real Postgres database call [`test_db`]. Each call gets a
//! **fresh, uniquely-named schema** with the full app schema synced into it, so
//! tests are fully isolated from each other and can run in parallel.
//!
//! ```ignore
//! #[tokio::test]
//! async fn records_a_queued_row() {
//!     let Some(db) = crate::test_support::db::test_db().await else {
//!         return; // no database available; skip
//!     };
//!     // ... use db.handle()
//! }
//! ```
//!
//! ## Requirements
//!
//! The harness needs a Postgres connection with permission to **create and drop
//! schemas**. It reads `DATABASE_URL` if set, otherwise assembles a URL from the
//! `POSTGRES_*` env vars (which `config_local.env` already provides).
//!
//! If no database is reachable, [`test_db`] returns `None` and the test is
//! skipped — so `cargo test` stays green on machines without Postgres.
//!
//! ## Why a schema per test (and not `sqlx::test` or `MockDatabase`)
//!
//! - **Not `MockDatabase`:** it asserts on the SQL you *expect* to emit, rather
//!   than exercising real queries. Brittle, and it wouldn't catch a wrong query.
//! - **Not `sqlx::test`:** it wants a `migrations/` folder. This project creates
//!   its schema via sea-orm's `schema-sync` at startup, so there are no
//!   migrations to apply. A schema-per-test harness reuses `schema-sync` instead
//!   of duplicating it.
//! - **Not transaction-rollback:** `DbHandle` holds a concrete
//!   `DatabaseConnection`, and a `DatabaseTransaction` isn't one. Making it
//!   generic over the executor would ripple through every repository.

use sea_orm::DatabaseConnection;
use sqlx::postgres::PgPoolOptions;

use crate::database::DbHandle;

/// A throwaway database schema for a single test.
///
/// Drops its schema on `Drop` (best-effort). If the process dies first, the
/// next call to [`test_db`] sweeps stale schemas.
pub struct TestDb {
    handle: DbHandle,
    schema: String,
    admin_url: String,
}

impl TestDb {
    /// A handle to the isolated schema. Cheap to clone.
    pub fn handle(&self) -> DbHandle {
        self.handle.clone()
    }

    /// The name of the isolated schema (useful for debugging).
    pub fn schema(&self) -> &str {
        &self.schema
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // `Drop` can't be async, so hand the cleanup to the runtime. Best-effort:
        // if the runtime is already shutting down we simply leave the schema for
        // the next run's sweep.
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };

        let url = self.admin_url.clone();
        let schema = self.schema.clone();

        rt.spawn(async move {
            if let Ok(pool) = PgPoolOptions::new().max_connections(1).connect(&url).await {
                let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                    .execute(&pool)
                    .await;
            }
        });
    }
}

/// Create an isolated test database, or `None` if no database is available.
///
/// Returning `None` (rather than panicking) lets DB tests skip gracefully on
/// machines without Postgres, keeping `cargo test` green everywhere.
pub async fn test_db() -> Option<TestDb> {
    let url = database_url()?;

    let pool = match PgPoolOptions::new().max_connections(1).connect(&url).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("test_support::db: skipping DB test, cannot connect: {err}");
            return None;
        }
    };

    // Sweep schemas left behind by a previous run that died before cleanup.
    sweep_stale_schemas(&pool).await;

    let schema = format!("test_{}", uuid::Uuid::new_v4().simple());

    if let Err(err) = sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&pool)
        .await
    {
        eprintln!(
            "test_support::db: skipping DB test, cannot create schema (does the DB user \
             have CREATE permission?): {err}"
        );
        return None;
    }

    let conn = match connect_scoped(&url, &schema).await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("test_support::db: skipping DB test, schema sync failed: {err}");
            let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                .execute(&pool)
                .await;
            return None;
        }
    };

    Some(TestDb {
        handle: DbHandle::new(conn),
        schema,
        admin_url: url,
    })
}

/// Connect with `search_path` pinned to `schema`, then sync the app schema into
/// it. Pinning via the connection URL's `options` applies to every pooled
/// connection, which is what makes this reliable.
async fn connect_scoped(url: &str, schema: &str) -> anyhow::Result<DatabaseConnection> {
    let separator = if url.contains('?') { '&' } else { '?' };
    let scoped_url = format!("{url}{separator}options=-csearch_path%3D{schema}");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&scoped_url)
        .await?;

    let conn = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool);

    conn.get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await?;

    Ok(conn)
}

/// Drop any `test_*` schemas left over from a previous, uncleanly-terminated run.
async fn sweep_stale_schemas(pool: &sqlx::PgPool) {
    let stale: Vec<String> = match sqlx::query_scalar(
        "SELECT schema_name FROM information_schema.schemata WHERE schema_name LIKE 'test\\_%'",
    )
    .fetch_all(pool)
    .await
    {
        Ok(names) => names,
        Err(_) => return,
    };

    for schema in stale {
        let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(pool)
            .await;
    }
}

/// Resolve a Postgres URL from the environment.
///
/// Prefers `DATABASE_URL` (the sqlx convention); falls back to the `POSTGRES_*`
/// vars the app already uses.
fn database_url() -> Option<String> {
    if let Ok(url) = std::env::var("DATABASE_URL")
        && !url.is_empty()
    {
        return Some(url);
    }

    let user = std::env::var("POSTGRES_USER").ok()?;
    let password = std::env::var("POSTGRES_PASSWORD").ok()?;
    let host_port = std::env::var("POSTGRES_HOST_PORT").ok()?;
    let db = std::env::var("POSTGRES_DB").ok()?;

    Some(format!("postgres://{user}:{password}@{host_port}/{db}"))
}
