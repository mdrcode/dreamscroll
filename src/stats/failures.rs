//! Granular failure listing for a queue.
//!
//! Windows are `created_at` cohorts (same as the timing stats). The default is
//! terminal failures (`status_code = 5`); `include_retrying` adds runs currently
//! in `ErrorWillRetry`, which is where a retry storm is visible before terminalization.
//! The row age is measured from `updated_at` (when the failure was recorded).

use anyhow::Context;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde::{Deserialize, Serialize};

use crate::{database, task};

use super::queue_label;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueFailure {
    pub queue: String,
    pub task_type: String,
    pub entity_type: String,
    pub entity_id: i32,
    pub run_id: String,
    pub attempts: i32,
    /// `complete_failure` or `error_will_retry`.
    pub status: String,
    pub error_code: Option<i32>,
    pub error_message: Option<String>,
    /// Duration of the most recent failed attempt.
    pub duration_ms: Option<i64>,
    /// Time since the failure was recorded.
    pub age_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueFailuresResponse {
    pub hours: i64,
    pub include_retrying: bool,
    pub failures: Vec<QueueFailure>,
}

const SELECT: &str = r#"
SELECT
  task_type,
  entity_type,
  entity_id,
  run_id,
  attempts,
  status_code,
  last_error_code,
  last_error_message,
  last_error_duration_ms,
  (EXTRACT(EPOCH FROM (now() - updated_at)) * 1000.0)::bigint AS age_ms
FROM task_runs
WHERE created_at >= now() - make_interval(hours => $1::int)
  AND status_code = 5
ORDER BY updated_at DESC
LIMIT $2
"#;

const SELECT_WITH_RETRYING: &str = r#"
SELECT
  task_type,
  entity_type,
  entity_id,
  run_id,
  attempts,
  status_code,
  last_error_code,
  last_error_message,
  last_error_duration_ms,
  (EXTRACT(EPOCH FROM (now() - updated_at)) * 1000.0)::bigint AS age_ms
FROM task_runs
WHERE created_at >= now() - make_interval(hours => $1::int)
  AND status_code IN (3, 5)
ORDER BY updated_at DESC
LIMIT $2
"#;

pub async fn queue_failures(
    db: &database::DbHandle,
    hours: i64,
    limit: i64,
    include_retrying: bool,
) -> anyhow::Result<QueueFailuresResponse> {
    let sql = if include_retrying {
        SELECT_WITH_RETRYING
    } else {
        SELECT
    };
    let rows = db
        .conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            vec![hours.into(), limit.into()],
        ))
        .await
        .context("query queue failures")?;

    let failures = rows
        .iter()
        .map(|row| {
            let task_type: String = row.try_get_by("task_type")?;
            let status_code: i32 = row.try_get_by("status_code")?;
            Ok(QueueFailure {
                queue: queue_label(&task_type),
                status: task::TaskRunStatus::from_i32(status_code)?
                    .as_str()
                    .to_string(),
                task_type,
                entity_type: row.try_get_by("entity_type")?,
                entity_id: row.try_get_by("entity_id")?,
                run_id: row.try_get_by("run_id")?,
                attempts: row.try_get_by("attempts")?,
                error_code: row.try_get_by("last_error_code")?,
                error_message: row.try_get_by("last_error_message")?,
                duration_ms: row.try_get_by("last_error_duration_ms")?,
                age_ms: row.try_get_by("age_ms")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(QueueFailuresResponse {
        hours,
        include_retrying,
        failures,
    })
}
