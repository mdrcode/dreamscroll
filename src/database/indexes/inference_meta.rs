use sea_orm::{ConnectionTrait, DbErr};

pub async fn after_schema_sync<C: ConnectionTrait>(conn: &C) -> Result<(), DbErr> {
    conn.execute_unprepared(
        "CREATE INDEX IF NOT EXISTS \"idx-inference_metas-task_attempt\" \
         ON inference_metas (task_run_id, attempt)",
    )
    .await?;
    Ok(())
}
