use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement, TransactionTrait,
};

use super::indexes;

// Bump for model or manual-index changes that require an explicit schema sync.
const REQUIRED_SCHEMA_VERSION: i32 = 8;

/// Reconcile the model schema, manual indexes, and applied version atomically.
pub async fn sync_schema(conn: &DatabaseConnection) -> anyhow::Result<()> {
    let registry = conn.get_schema_registry("dreamscroll::model::*");

    conn.transaction::<_, (), DbErr>(|transaction| {
        Box::pin(async move {
            transaction
                .execute_unprepared("SELECT pg_advisory_xact_lock(20261016, 1)")
                .await?;
            let marker_table = transaction
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT to_regclass('dreamscroll_schema_version')::text",
                ))
                .await?
                .expect("to_regclass always returns one row");
            let marker_table_name: Option<String> = marker_table.try_get_by_index(0)?;
            if marker_table_name.is_some() {
                if let Some(marker) = transaction
                    .query_one_raw(Statement::from_string(
                        DatabaseBackend::Postgres,
                        "SELECT version FROM dreamscroll_schema_version WHERE id = 1",
                    ))
                    .await?
                {
                    let applied_version: i32 = marker.try_get_by_index(0)?;
                    if applied_version > REQUIRED_SCHEMA_VERSION {
                        return Err(DbErr::Custom(format!(
                            "database schema version {applied_version} is newer than this binary's required version {REQUIRED_SCHEMA_VERSION}; refusing to sync"
                        )));
                    }
                }
            }

            indexes::task_run::before_schema_sync(transaction).await?;
            registry.sync(transaction).await?;
            indexes::task_run::after_schema_sync(transaction).await?;
            indexes::inference_meta::after_schema_sync(transaction).await?;
            transaction
                .execute_unprepared(
                    "CREATE TABLE IF NOT EXISTS dreamscroll_schema_version (\
                        id SMALLINT PRIMARY KEY CHECK (id = 1), \
                        version INTEGER NOT NULL\
                    )",
                )
                .await?;
            transaction
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "INSERT INTO dreamscroll_schema_version (id, version) VALUES (1, $1) \
                     ON CONFLICT (id) DO UPDATE SET version = EXCLUDED.version",
                    [REQUIRED_SCHEMA_VERSION.into()],
                ))
                .await?;
            Ok(())
        })
    })
    .await?;

    tracing::info!(
        schema_version = REQUIRED_SCHEMA_VERSION,
        "Synchronized Postgres schema"
    );
    Ok(())
}

/// Reject an application start when its minimum required schema was not applied.
pub async fn check_schema_version(conn: &DatabaseConnection) -> anyhow::Result<()> {
    let table = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT to_regclass('dreamscroll_schema_version')::text",
        ))
        .await?
        .expect("to_regclass always returns one row");
    let table_name: Option<String> = table.try_get_by_index(0)?;
    if table_name.is_none() {
        anyhow::bail!("database schema version is missing; run `dreamscroll_db sync_schema`");
    }

    let row = conn
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT version FROM dreamscroll_schema_version WHERE id = 1",
        ))
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "database schema version row is missing; run `dreamscroll_db sync_schema`"
            )
        })?;
    let applied_version: i32 = row.try_get_by_index(0)?;
    if applied_version < REQUIRED_SCHEMA_VERSION {
        anyhow::bail!(
            "database schema version {applied_version} is below required version {REQUIRED_SCHEMA_VERSION}; run `dreamscroll_db sync_schema`"
        );
    }

    tracing::info!(
        required_schema_version = REQUIRED_SCHEMA_VERSION,
        applied_schema_version = applied_version,
        "Database schema is compatible"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, RuntimeErr, Statement};

    use super::{check_schema_version, sync_schema};

    #[tokio::test]
    async fn repeated_schema_sync_preserves_backfill_task_uniqueness() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let conn = test_db.handle().conn;

        sync_schema(&conn)
            .await
            .expect("first explicit schema sync should succeed");
        sync_schema(&conn)
            .await
            .expect("repeated explicit schema sync should succeed");
        check_schema_version(&conn)
            .await
            .expect("sync should record a compatible schema version");

        let index = conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT indexdef FROM pg_indexes \
             WHERE schemaname = current_schema() \
             AND indexname = 'idx-task_runs-backfill_task'",
            ))
            .await
            .expect("querying the backfill index should succeed")
            .expect("backfill unique index should exist");
        let index_definition: String = index
            .try_get_by_index(0)
            .expect("index definition should be text");
        assert!(index_definition.starts_with("CREATE UNIQUE INDEX"));
        assert!(index_definition.contains("(backfill_id, logical_id)"));

        insert_task_run(&conn, 1, "schema-sync-run-1")
            .await
            .expect("first tagged run should insert");
        let duplicate = insert_task_run(&conn, 2, "schema-sync-run-2")
            .await
            .expect_err("same backfill/logical task must remain unique across run numbers");
        let database_error = match &duplicate {
            DbErr::Exec(RuntimeErr::SqlxError(error))
            | DbErr::Query(RuntimeErr::SqlxError(error)) => error
                .as_database_error()
                .expect("duplicate should be a PostgreSQL database error"),
            error => panic!("expected unique violation, got {error}"),
        };
        assert_eq!(database_error.code().as_deref(), Some("23505"));
        assert_eq!(
            database_error.constraint(),
            Some("idx-task_runs-backfill_task")
        );
    }

    #[tokio::test]
    async fn schema_version_guard_rejects_stale_and_missing_markers() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let conn = test_db.handle().conn;

        check_schema_version(&conn)
            .await
            .expect("synced schema version should be accepted");
        conn.execute_unprepared(&format!(
            "UPDATE dreamscroll_schema_version SET version = {} WHERE id = 1",
            super::REQUIRED_SCHEMA_VERSION + 1
        ))
        .await
        .expect("advancing the test marker should succeed");
        check_schema_version(&conn)
            .await
            .expect("a newer compatible schema version should be accepted");

        conn.execute_unprepared("UPDATE dreamscroll_schema_version SET version = 0 WHERE id = 1")
            .await
            .expect("downgrading the test marker should succeed");
        let stale_error = check_schema_version(&conn)
            .await
            .expect_err("an older schema marker must fail the startup check");
        assert!(stale_error.to_string().contains("below required version"));
        assert!(
            stale_error
                .to_string()
                .contains("dreamscroll_db sync_schema")
        );

        conn.execute_unprepared("DROP TABLE dreamscroll_schema_version")
            .await
            .expect("dropping the test marker should succeed");
        let missing_error = check_schema_version(&conn)
            .await
            .expect_err("a missing schema marker must fail the startup check");
        assert!(
            missing_error
                .to_string()
                .contains("schema version is missing")
        );
        assert!(
            missing_error
                .to_string()
                .contains("dreamscroll_db sync_schema")
        );
    }

    #[tokio::test]
    async fn schema_sync_drops_manual_indexes_only_in_current_schema() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let conn = test_db.handle().conn;

        let current_schema = conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT current_schema()",
            ))
            .await
            .expect("reading the current schema should succeed")
            .expect("current_schema should return one row");
        let current_schema: String = current_schema
            .try_get_by_index(0)
            .expect("current schema should be text");
        let other_schema = format!("test_index_scope_{}", uuid::Uuid::new_v4().simple());

        conn.execute_unprepared(&format!("CREATE SCHEMA {other_schema}"))
            .await
            .expect("creating the other schema should succeed");
        conn.execute_unprepared(&format!(
            "CREATE TABLE {other_schema}.task_runs (backfill_id TEXT, logical_id TEXT)"
        ))
        .await
        .expect("creating the other task_runs table should succeed");
        conn.execute_unprepared(&format!(
            "CREATE UNIQUE INDEX \"idx-task_runs-backfill_task\" \
             ON {other_schema}.task_runs (backfill_id, logical_id)"
        ))
        .await
        .expect("creating the same-named index in the other schema should succeed");

        conn.execute_unprepared("DROP INDEX IF EXISTS \"idx-task_runs-backfill_task\"")
            .await
            .expect("dropping the current schema index should succeed");
        conn.execute_unprepared(&format!(
            "SET search_path TO {current_schema}, {other_schema}"
        ))
        .await
        .expect("setting the multi-schema search path should succeed");
        sync_schema(&conn)
            .await
            .expect("schema sync should succeed with a multi-schema search path");

        let index = conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (SELECT 1 FROM pg_indexes \
                 WHERE schemaname = $1 AND indexname = $2)",
                [
                    other_schema.as_str().into(),
                    "idx-task_runs-backfill_task".into(),
                ],
            ))
            .await
            .expect("checking the other schema index should succeed")
            .expect("EXISTS should return one row");
        let exists: bool = index
            .try_get_by_index(0)
            .expect("EXISTS result should be boolean");
        assert!(exists, "schema sync must preserve the other schema's index");

        conn.execute_unprepared(&format!("DROP SCHEMA {other_schema} CASCADE"))
            .await
            .expect("cleaning up the other schema should succeed");
    }

    #[tokio::test]
    async fn schema_sync_refuses_to_downgrade_a_newer_marker() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let conn = test_db.handle().conn;

        conn.execute_unprepared(&format!(
            "UPDATE dreamscroll_schema_version SET version = {} WHERE id = 1",
            super::REQUIRED_SCHEMA_VERSION + 1
        ))
        .await
        .expect("advancing the marker should succeed");
        let error = sync_schema(&conn)
            .await
            .expect_err("an older binary must not sync over a newer schema marker");
        assert!(
            error
                .to_string()
                .contains("newer than this binary's required version")
        );

        let marker = conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT version FROM dreamscroll_schema_version WHERE id = 1",
            ))
            .await
            .expect("reading the schema marker should succeed")
            .expect("the schema marker should exist");
        let version: i32 = marker
            .try_get_by_index(0)
            .expect("schema marker version should be an integer");
        assert_eq!(version, super::REQUIRED_SCHEMA_VERSION + 1);
        check_schema_version(&conn)
            .await
            .expect("newer schema markers should remain compatible at startup");
    }

    async fn insert_task_run(
        conn: &sea_orm::DatabaseConnection,
        run_number: i32,
        run_id: &str,
    ) -> Result<(), DbErr> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO task_runs \
            (logical_id, run_id, run_number, backfill_id, task_type, entity_owner_id, \
             entity_type, entity_id, status_code, attempts) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            [
                "schema-sync-logical".into(),
                run_id.into(),
                run_number.into(),
                "schema-sync-backfill".into(),
                "test".into(),
                1.into(),
                "capture".into(),
                1.into(),
                0.into(),
                0.into(),
            ],
        ))
        .await?;
        Ok(())
    }
}
