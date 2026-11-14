//! Schema-pooled database harness for DB tests.
//!
//! Tests that need a real Postgres database call [`test_db`]. It hands back a schema of its own,
//! with the full app schema synced into it, so tests are fully isolated from each other and can run
//! in parallel.
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
//! ## Why schemas are pooled
//!
//! Building a schema costs ~115ms: `CREATE SCHEMA`, a fresh pool, and `sync_schema` over 21 tables.
//! A suite with ~126 DB tests was spending essentially all of its wall time there, and because
//! creating and dropping schemas takes catalog locks, the tests did not scale: eight threads ran
//! slower than the default.
//!
//! So a schema is built once, used by test after test, and reset in between by a single `TRUNCATE`
//! over every table (~21ms, no catalog locks). [`TestDb`] returns its schema to the pool in `Drop` —
//! synchronously, because the old teardown handed the `DROP SCHEMA` to the test's own runtime, which
//! died first, so *every* schema leaked and every run paid to sweep the previous run's 126.
//!
//! The pool grows to the number of tests that are running at once and no further. A returned
//! schema is reset when it is next handed out, not when it comes back, so a panicking test still
//! returns its schema (and the open transaction it left is rolled back by the reset).
//!
//! ## Requirements
//!
//! The harness needs a Postgres connection with permission to **create and drop schemas**. It loads
//! the app's config (`config_local.env` + `.env`) and builds the connection URL with
//! [`crate::database::make_url_from_config`], exactly as the app does — so if the app can reach
//! Postgres, so can the tests.
//!
//! If no database is reachable, [`test_db`] returns `None` and the test is skipped — so `cargo test`
//! stays green on machines without Postgres.
//!
//! ## What a test may not do
//!
//! The truncate list is the tables `sync_schema` created, captured once per schema. A test that
//! creates its own tables leaves them — and their rows — for the next test that draws that schema.
//! Tests also share a connection, so anything that outlives the test (a detached task still writing)
//! now lands in a schema that will be reused. Neither has bitten yet; both are why this is documented
//! rather than assumed.
//!
//! ## Why a schema per test
//!
//! See `plan/testing.md` for why this beats `MockDatabase`, `sqlx::test`, and transaction-rollback.

use std::sync::{LazyLock, Mutex};

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
};

use crate::{config, database, test_support};

/// A schema checked out of the pool for the lifetime of one test.
///
/// Returns its schema to the pool on `Drop`. The `handle` belongs to this test's runtime.
pub struct TestDb {
    handle: database::DbHandle,
    slot: Option<Slot>,
}

/// A reusable schema, and the tables to empty before it is handed out again.
///
/// It carries no connection: a `DatabaseConnection` belongs to the runtime that created it, and
/// every `#[tokio::test]` brings its own. Reusing a pool across runtimes deadlocks — the pool's
/// background tasks died with the first runtime — so each test connects for itself.
struct Slot {
    schema: String,
    tables: Vec<String>,
    /// The schema-version marker's value at build time. Truncating the tables takes the marker with
    /// them, and a schema without one makes `sync_schema` refuse to run — so it is put back.
    schema_version: Option<i32>,
}

/// The schemas currently free for reuse. Grows to the peak number of concurrent tests.
static POOLED: LazyLock<Mutex<Vec<Slot>>> = LazyLock::new(|| Mutex::new(Vec::new()));

impl TestDb {
    /// A handle on the schema. The `TestDb` owns it: keep it alive for the whole test. Dropping it
    /// returns the schema to the pool, where another test may check it out and truncate it while
    /// this one is still running — so never consume `TestDb` in an expression. `db_or_skip!(db)`
    /// exists to make that the easy thing to write.
    pub fn handle(&self) -> database::DbHandle {
        self.handle.clone()
    }

    /// The schema this handle has checked out. For diagnostics, and for the one test that has to
    /// name the row it is asserting about.
    pub fn schema(&self) -> &str {
        &self
            .slot
            .as_ref()
            .expect("a TestDb always holds a schema until it is dropped")
            .schema
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        // Synchronous on purpose: an async hand-off would be cancelled with the test's runtime.
        let Some(slot) = self.slot.take() else {
            return;
        };
        match POOLED.lock() {
            Ok(mut pool) => pool.push(slot),
            Err(poisoned) => poisoned.into_inner().push(slot),
        }
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

    sweep_stale_schemas(&cfg).await;

    let reused = match POOLED.lock() {
        Ok(mut pool) => pool.pop(),
        Err(poisoned) => poisoned.into_inner().pop(),
    };

    if let Some(slot) = reused {
        match connect_with_schema(&cfg, &slot.schema).await {
            Ok(conn) => match reset(&conn, &slot).await {
                Ok(()) => {
                    return Some(TestDb {
                        handle: database::DbHandle::new(conn),
                        slot: Some(slot),
                    });
                }
                Err(err) => {
                    eprintln!(
                        "test_support::test_db: resetting a pooled schema failed ({err}); replacing it"
                    );
                    drop_schema(&cfg, &slot.schema).await;
                }
            },
            Err(err) => eprintln!(
                "test_support::test_db: cannot connect to pooled schema {}: {err}",
                slot.schema
            ),
        }
    }

    // A reachable server is the only condition that may skip a DB test. Everything after this is
    // our own schema, so a failure is a bug and must fail the test rather than read as a pass — a
    // broken `sync_schema` once turned every DB test into a silent no-op.
    let base = match connect_base(&cfg).await {
        Ok(base) => base,
        Err(err) => {
            eprintln!("test_support::test_db: skipping DB test, no database: {err}");
            return None;
        }
    };

    match build_slot(&cfg, &base).await {
        Ok((slot, conn)) => Some(TestDb {
            handle: database::DbHandle::new(conn),
            slot: Some(slot),
        }),
        Err(err) => panic!("test_support::test_db: building a test schema failed: {err:#}"),
    }
}

/// Build a schema, sync the app schema into it, and learn the tables it may need to truncate.
async fn build_slot(
    cfg: &config::Config,
    base: &DatabaseConnection,
) -> anyhow::Result<(Slot, DatabaseConnection)> {
    let schema = format!(
        "test_{}_{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    );

    base.execute_unprepared(&format!("CREATE SCHEMA {schema}"))
        .await?;

    let built = async {
        let conn = connect_with_schema(cfg, &schema).await?;
        database::sync_schema(&conn).await?;
        let tables = tables_to_truncate(&conn, &schema).await?;
        let schema_version = schema_version(&conn).await?;
        Ok::<_, anyhow::Error>((tables, schema_version, conn))
    }
    .await;

    match built {
        Ok((tables, schema_version, conn)) => Ok((
            Slot {
                schema,
                tables,
                schema_version,
            },
            conn,
        )),
        Err(err) => {
            let _ = base
                .execute_unprepared(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                .await;
            Err(err)
        }
    }
}

/// Return a schema to a known-empty state.
///
/// A panicking test can leave a transaction open, and the next test would then inherit it, so the
/// reset rolls back first and re-pins the search path in case the test moved it. The table names are
/// schema-qualified, so the truncate does not depend on the search path either.
async fn reset(conn: &DatabaseConnection, slot: &Slot) -> anyhow::Result<()> {
    let _ = conn.execute_unprepared("ROLLBACK").await;
    conn.execute_unprepared(&format!("SET search_path TO \"{}\"", slot.schema))
        .await?;
    if !slot.tables.is_empty() {
        conn.execute_unprepared(&format!(
            "TRUNCATE {} RESTART IDENTITY CASCADE",
            slot.tables.join(", ")
        ))
        .await?;
    }
    if let Some(version) = slot.schema_version {
        conn.execute_unprepared(&format!(
            "INSERT INTO dreamscroll_schema_version (id, version) VALUES (1, {version}) \
             ON CONFLICT (id) DO UPDATE SET version = EXCLUDED.version"
        ))
        .await?;
    }
    Ok(())
}

async fn connect_base(cfg: &config::Config) -> anyhow::Result<DatabaseConnection> {
    Ok(Database::connect(database::make_url_from_config(cfg, None, false)).await?)
}

/// The version this schema was synced to, for [`reset`] to put back after a truncate.
async fn schema_version(conn: &DatabaseConnection) -> anyhow::Result<Option<i32>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT version FROM dreamscroll_schema_version WHERE id = 1".to_string(),
        ))
        .await?;
    rows.first()
        .map(|row| row.try_get_by_index::<i32>(0).map_err(Into::into))
        .transpose()
}

/// The base tables in `schema`, qualified and quoted, ready for a `TRUNCATE`.
async fn tables_to_truncate(
    conn: &DatabaseConnection,
    schema: &str,
) -> anyhow::Result<Vec<String>> {
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
            [schema.into()],
        ))
        .await?;

    rows.iter()
        .map(|row| {
            row.try_get_by_index::<String>(0)
                .map(|table| format!("\"{schema}\".\"{table}\""))
                .map_err(Into::into)
        })
        .collect()
}

/// Connect with the schema pinned, so subsequent Connections are isolated to
/// that schema.
async fn connect_with_schema(
    cfg: &config::Config,
    schema: &str,
) -> anyhow::Result<DatabaseConnection> {
    let url = database::make_url_from_config(cfg, Some(schema), false);
    // A pooled schema outlives the test that holds it, so keep its pool small: one connection is
    // what these tests use, and letting a hundred accumulate per schema would exhaust the server.
    let options = ConnectOptions::new(url)
        .max_connections(4)
        .min_connections(1)
        .to_owned();
    Ok(Database::connect(options).await?)
}

async fn drop_schema(cfg: &config::Config, schema: &str) {
    if let Ok(base) = connect_base(cfg).await {
        let _ = base
            .execute_unprepared(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await;
    }
}

/// Drop the schemas of test processes that are gone, and never those of one that is running.
///
/// Runs **once per test process**. Schemas are named `test_<pid>_<uuid>`, so a schema whose owner is
/// still alive belongs to a concurrent run and is left alone; anything unparseable predates the
/// naming and is swept.
async fn sweep_stale_schemas(cfg: &config::Config) {
    static SWEPT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

    SWEPT
        .get_or_init(|| async {
            let Ok(base) = connect_base(cfg).await else {
                return;
            };
            let rows = match base
                .query_all_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT schema_name::text FROM information_schema.schemata \
                     WHERE schema_name LIKE 'test\\_%'"
                        .to_string(),
                ))
                .await
            {
                Ok(rows) => rows,
                Err(err) => {
                    eprintln!("test_support::test_db: cannot sweep stale schemas: {err}");
                    return;
                }
            };

            for row in rows {
                let Ok(schema) = row.try_get_by_index::<String>(0) else {
                    continue;
                };
                if owner_pid(&schema).is_some_and(process_alive) {
                    continue;
                }
                drop_schema(cfg, &schema).await;
            }
        })
        .await;
}

/// The process that created a `test_<pid>_<uuid>` schema.
fn owner_pid(schema: &str) -> Option<u32> {
    schema
        .strip_prefix("test_")?
        .split('_')
        .next()?
        .parse()
        .ok()
}

/// Whether a process is still running. `kill -0` is the portable check that needs no argument.
fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this harness exists to prevent, from the other side: `Drop` used to hand the
    /// cleanup to the test's own runtime, which was already shutting down, so *every* schema leaked
    /// and each run swept the previous run's 126. Dropping a handle now returns its schema to the
    /// pool, so the next acquisition is handed the same one instead of building another.
    ///
    /// A concurrent test may draw the returned schema first, in which case it is still not leaked —
    /// hence the retries. Without the fix the schema is never in the pool and no retry can find it.
    #[tokio::test]
    async fn a_dropped_handle_returns_its_schema_to_the_pool() {
        let Some(first) = test_db().await else {
            return;
        };
        let schema = first.schema().to_string();
        drop(first);

        for _ in 0..8 {
            let Some(next) = test_db().await else {
                return;
            };
            if next.schema() == schema {
                return;
            }
        }

        panic!("schema {schema} was never handed back to the pool after its handle was dropped");
    }
}
