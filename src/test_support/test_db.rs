//! Schema-isolated database harness for DB tests.
//!
//! Tests that need a real Postgres database call [`test_db`]. Each call gets a
//! **fresh, uniquely-named schema** with the full app schema synced into it, so
//! tests are fully isolated from each other and can run in parallel.
//!
//! ```ignore
//! #[tokio::test]
//! async fn records_a_queued_row() {
//!     let Some(db) = crate::test_support::test_db::test_db().await else {
//!         return; // no database available; skip
//!     };
//!     // ... use db.handle()
//! }
//! ```
//!
//! ## Requirements
//!
//! The harness needs a Postgres connection with permission to **create and drop
//! schemas**. It loads the app's config (`config_local.env` + `.env`) and builds
//! the connection URL with [`crate::database::make_url_from_config`], exactly as
//! the app does — so if the app can reach Postgres, so can the tests.
//!
//! If no database is reachable, [`test_db`] returns `None` and the test is
//! skipped — so `cargo test` stays green on machines without Postgres.
//!
//! ## Why a schema per test
//!
//! See `_project/plans/testing.md` for why this beats `MockDatabase`,
//! `sqlx::test`, and transaction-rollback.

use sea_orm::DatabaseConnection;
use sqlx::postgres::PgPoolOptions;

use crate::{config, database, test_support};

/// Throwaway database schema for a single test.
///
/// Drops its schema on `Drop` (best-effort). If the process dies first, the
/// next test process sweeps stale schemas.
pub struct TestDb {
    handle: database::DbHandle,
    base_url: String, // no schema pinned, so that schemas can be created/dropped
    test_schema: String,
}

impl TestDb {
    pub fn handle(&self) -> database::DbHandle {
        self.handle.clone()
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

        let base_url = self.base_url.clone();
        let test_schema = self.test_schema.clone();

        rt.spawn(async move {
            if let Ok(pool) = PgPoolOptions::new()
                .max_connections(1)
                .connect(&base_url)
                .await
            {
                let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {test_schema} CASCADE"))
                    .execute(&pool)
                    .await;
            }
        });
    }
}

/// Create an isolated test schema, or `None` if no database is available.
///
/// Returning `None` (rather than panicking) lets DB tests skip gracefully on
/// machines without Postgres.
pub async fn test_db() -> Option<TestDb> {
    let cfg = match test_support::test_config::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("test_support::test_db: skipping DB test, cannot load config: {err}");
            return None;
        }
    };

    // Connect to the base Postgres URL (no schema pinned)
    let base_url = database::make_url_from_config(&cfg, None, false);
    let base_pool = match PgPoolOptions::new()
        .max_connections(1)
        .connect(&base_url)
        .await
    {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("test_support::test_db: skipping DB test, cannot connect: {err}");
            return None;
        }
    };

    // Sweep stale schemas (no-op after the first call in this process).
    sweep_stale_schemas(&base_pool).await;

    // Create the new, "current" test schema
    let new_test_schema = format!("test_{}", uuid::Uuid::new_v4().simple());
    if let Err(err) = sqlx::query(&format!("CREATE SCHEMA {new_test_schema}"))
        .execute(&base_pool)
        .await
    {
        eprintln!(
            "test_support::test_db: skipping DB test, cannot create new schema (does the DB user \
             have CREATE permission?): {err}"
        );
        return None;
    }

    let conn = match connect_with_schema(&cfg, &new_test_schema).await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("test_support::test_db: skipping DB test, cannot connect: {err}");
            let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {new_test_schema} CASCADE"))
                .execute(&base_pool)
                .await;
            return None;
        }
    };

    // Sync the app's models into the new schema
    if let Err(err) = conn
        .get_schema_registry("dreamscroll::model::*")
        .sync(&conn)
        .await
    {
        eprintln!("test_support::test_db: skipping DB test, schema sync failed: {err}");
        let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {new_test_schema} CASCADE"))
            .execute(&base_pool)
            .await;
        return None;
    }

    Some(TestDb {
        handle: database::DbHandle::new(conn),
        test_schema: new_test_schema,
        base_url,
    })
}

/// Connect with the schema pinned, so subsequent Connections are isolated to
/// that schema.
async fn connect_with_schema(
    cfg: &config::Config,
    schema: &str,
) -> anyhow::Result<DatabaseConnection> {
    let url_with_schema = database::make_url_from_config(cfg, Some(schema), false);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url_with_schema)
        .await?;

    let conn = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool);

    Ok(conn)
}

/// Drop any `test_*` schemas left over from a previous, uncleanly-terminated run.
///
/// Runs **once per test process**; later calls are no-ops.
///
/// NOTE WARN Known Issue: two test processes running concurrently against the
/// same database can sweep each other's live schemas.
async fn sweep_stale_schemas(pool: &sqlx::PgPool) {
    static SWEPT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    SWEPT
        .get_or_init(|| async {
            let stale: Vec<String> = match sqlx::query_scalar(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name LIKE 'test\\_%'",
            )
            .fetch_all(pool)
            .await
            {
                Ok(names) => names,
                Err(err) => {
                    eprintln!("test_support::test_db: cannot sweep stale schemas: {err}");
                    return;
                }
            };

            for schema in stale {
                let _ = sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                    .execute(pool)
                    .await;
            }
        })
        .await;
}
