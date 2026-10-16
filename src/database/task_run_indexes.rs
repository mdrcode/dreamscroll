use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

/// SeaORM's derive supports only one composite unique key per column, so these
/// indexes supplement registry synchronization without replacing the run ordinal key.
/// PostgreSQL's default NULLS DISTINCT behavior leaves ordinary reruns unchanged.
pub async fn sync_task_run_indexes(conn: &DatabaseConnection) -> Result<(), DbErr> {
    for sql in [
        "CREATE UNIQUE INDEX IF NOT EXISTS \"idx-task_runs-backfill_task\" ON task_runs (backfill_id, logical_id)",
        "CREATE INDEX IF NOT EXISTS \"idx-task_runs-backfill_status\" ON task_runs (backfill_id, task_type, status_code)",
    ] {
        conn.execute_raw(Statement::from_string(DbBackend::Postgres, sql))
            .await?;
    }
    Ok(())
}
