use sea_orm::{ConnectionTrait, DbErr};

struct ManualIndexOperation {
    drop_before_schema_sync: Option<&'static str>,
    create: &'static str,
}

const TASK_RUN_INDEXES: &[ManualIndexOperation] = &[
    // NULLS DISTINCT keeps untagged ordinary reruns outside the backfill-only uniqueness rule.
    ManualIndexOperation {
        drop_before_schema_sync: Some(
            r#"DO $sync$
BEGIN
    EXECUTE format(
        'DROP INDEX IF EXISTS %I.%I',
        current_schema(),
        'idx-task_runs-backfill_task'
    );
END;
$sync$;"#,
        ),
        create: "CREATE UNIQUE INDEX IF NOT EXISTS \"idx-task_runs-backfill_task\" ON task_runs (backfill_id, logical_id)",
    },
    ManualIndexOperation {
        drop_before_schema_sync: None,
        create: "CREATE INDEX IF NOT EXISTS \"idx-task_runs-backfill_status\" ON task_runs (backfill_id, task_type, status_code)",
    },
];

/// Remove custom unique indexes that SeaORM 2.0.4 misclassifies as constraints.
pub async fn before_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    for index in TASK_RUN_INDEXES {
        if let Some(sql) = index.drop_before_schema_sync {
            conn.execute_unprepared(sql).await?;
        }
    }
    Ok(())
}

/// Ensure application-managed TaskRun indexes after entity schema sync.
pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    for index in TASK_RUN_INDEXES {
        conn.execute_unprepared(index.create).await?;
    }
    Ok(())
}
