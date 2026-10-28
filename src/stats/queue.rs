//! Windowed queue timing and backlog.
//!
//! Timing columns are last-attempt values: `processing_started_at` and the
//! duration columns are overwritten on retry, so retried runs contribute their
//! final attempt. Windows are `created_at` cohorts: a run created before the
//! window but finishing inside it is excluded. Failed inference calls leave no
//! `inference_metas` row; the failure side of these stats is the authoritative
//! view of that tail. `ErrorWillRetry` rows are included in the snapshot with
//! their age measured from `updated_at`, the moment the failure was recorded.

use std::collections::HashMap;

use anyhow::Context;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};
use serde::{Deserialize, Serialize};

use crate::{config, database};

use super::{FailureCodeCount, MetricStats, queue_label};

/// One task type within a window.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueStats {
    pub task_type: String,
    pub queue: String,
    pub queued_success: MetricStats,
    pub queued_failure: MetricStats,
    pub processing_success: MetricStats,
    pub processing_failure: MetricStats,
    pub failure_codes: Vec<FailureCodeCount>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueWindow {
    pub hours: i64,
    pub queues: Vec<QueueStats>,
}

/// Current (unwindowed) backlog ages.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueBacklogRow {
    pub task_type: String,
    pub queue: String,
    pub queued: MetricStats,
    pub in_progress: MetricStats,
    /// Failed with retries remaining; age is time since the failure was recorded.
    pub error_will_retry: MetricStats,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueueStatsResponse {
    /// True when the queue backend cannot resume in-memory work (the local
    /// backend): stale Queued/InProgress rows make the snapshot misleading, so
    /// the CLI suppresses it.
    pub snapshot_suppressed: bool,
    pub backlog: Vec<QueueBacklogRow>,
    pub windows: Vec<QueueWindow>,
}

const QUEUE_STATS_SQL: &str = r#"
WITH base AS (
  SELECT
    task_type,
    status_code,
    EXTRACT(EPOCH FROM (processing_started_at - created_at)) * 1000.0 AS queued_ms,
    success_duration_ms::double precision AS success_ms,
    last_error_duration_ms::double precision AS failure_ms
  FROM task_runs
  WHERE created_at >= now() - make_interval(hours => $1::int)
)
SELECT
  task_type,
  COUNT(*) FILTER (WHERE status_code = 4 AND queued_ms IS NOT NULL) AS qs_n,
  (AVG(queued_ms) FILTER (WHERE status_code = 4))::double precision AS qs_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 4) AS qs_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 4) AS qs_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 4) AS qs_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 4) AS qs_p99,
  COUNT(*) FILTER (WHERE status_code = 5 AND queued_ms IS NOT NULL) AS qf_n,
  (AVG(queued_ms) FILTER (WHERE status_code = 5))::double precision AS qf_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 5) AS qf_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 5) AS qf_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 5) AS qf_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY queued_ms) FILTER (WHERE status_code = 5) AS qf_p99,
  COUNT(*) FILTER (WHERE status_code = 4 AND success_ms IS NOT NULL) AS ps_n,
  (AVG(success_ms) FILTER (WHERE status_code = 4))::double precision AS ps_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY success_ms) FILTER (WHERE status_code = 4) AS ps_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY success_ms) FILTER (WHERE status_code = 4) AS ps_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY success_ms) FILTER (WHERE status_code = 4) AS ps_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY success_ms) FILTER (WHERE status_code = 4) AS ps_p99,
  COUNT(*) FILTER (WHERE status_code = 5 AND failure_ms IS NOT NULL) AS pf_n,
  (AVG(failure_ms) FILTER (WHERE status_code = 5))::double precision AS pf_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY failure_ms) FILTER (WHERE status_code = 5) AS pf_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY failure_ms) FILTER (WHERE status_code = 5) AS pf_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY failure_ms) FILTER (WHERE status_code = 5) AS pf_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY failure_ms) FILTER (WHERE status_code = 5) AS pf_p99
FROM base
GROUP BY task_type
ORDER BY task_type
"#;

const BACKLOG_SQL: &str = r#"
WITH base AS (
  SELECT
    task_type,
    status_code,
    CASE
      WHEN status_code = 1 THEN EXTRACT(EPOCH FROM (now() - created_at)) * 1000.0
      WHEN status_code = 2 THEN EXTRACT(EPOCH FROM (now() - processing_started_at)) * 1000.0
      WHEN status_code = 3 THEN EXTRACT(EPOCH FROM (now() - updated_at)) * 1000.0
    END AS age_ms
  FROM task_runs
  WHERE status_code IN (1, 2, 3)
)
SELECT
  task_type,
  COUNT(*) FILTER (WHERE status_code = 1 AND age_ms IS NOT NULL) AS q_n,
  (AVG(age_ms) FILTER (WHERE status_code = 1))::double precision AS q_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 1) AS q_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 1) AS q_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 1) AS q_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 1) AS q_p99,
  COUNT(*) FILTER (WHERE status_code = 2 AND age_ms IS NOT NULL) AS p_n,
  (AVG(age_ms) FILTER (WHERE status_code = 2))::double precision AS p_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 2) AS p_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 2) AS p_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 2) AS p_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 2) AS p_p99,
  COUNT(*) FILTER (WHERE status_code = 3 AND age_ms IS NOT NULL) AS r_n,
  (AVG(age_ms) FILTER (WHERE status_code = 3))::double precision AS r_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 3) AS r_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 3) AS r_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 3) AS r_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY age_ms) FILTER (WHERE status_code = 3) AS r_p99
FROM base
GROUP BY task_type
ORDER BY task_type
"#;

const FAILURE_CODES_SQL: &str = r#"
SELECT task_type, last_error_code AS code, COUNT(*)::bigint AS count
FROM task_runs
WHERE created_at >= now() - make_interval(hours => $1::int)
  AND status_code = 5
GROUP BY task_type, last_error_code
ORDER BY task_type, count DESC
"#;

pub async fn queue_stats(
    db: &database::DbHandle,
    windows: &[i64],
    task_backend: config::TaskQueueBackend,
) -> anyhow::Result<QueueStatsResponse> {
    let backlog = load_backlog(db).await?;

    let mut window_stats = Vec::with_capacity(windows.len());
    for &hours in windows {
        let rows = db
            .conn
            .query_all_raw(stmt(QUEUE_STATS_SQL, hours))
            .await
            .context("query queue timing stats")?;

        let codes = load_failure_codes(db, hours).await?;
        let mut queues = Vec::with_capacity(rows.len());
        for row in &rows {
            let task_type: String = row.try_get_by("task_type")?;
            queues.push(QueueStats {
                queue: queue_label(&task_type),
                failure_codes: codes.get(&task_type).cloned().unwrap_or_default(),
                task_type,
                queued_success: metric(row, "qs")?,
                queued_failure: metric(row, "qf")?,
                processing_success: metric(row, "ps")?,
                processing_failure: metric(row, "pf")?,
            });
        }

        window_stats.push(QueueWindow { hours, queues });
    }

    Ok(QueueStatsResponse {
        snapshot_suppressed: task_backend == config::TaskQueueBackend::Local,
        backlog,
        windows: window_stats,
    })
}

async fn load_backlog(db: &database::DbHandle) -> anyhow::Result<Vec<QueueBacklogRow>> {
    let rows = db
        .conn
        .query_all_raw(stmt(BACKLOG_SQL, 0))
        .await
        .context("query queue backlog")?;
    rows.iter()
        .map(|row| {
            let task_type: String = row.try_get_by("task_type")?;
            Ok(QueueBacklogRow {
                queue: queue_label(&task_type),
                task_type,
                queued: metric(row, "q")?,
                in_progress: metric(row, "p")?,
                error_will_retry: metric(row, "r")?,
            })
        })
        .collect()
}

async fn load_failure_codes(
    db: &database::DbHandle,
    hours: i64,
) -> anyhow::Result<HashMap<String, Vec<FailureCodeCount>>> {
    let rows = db
        .conn
        .query_all_raw(stmt(FAILURE_CODES_SQL, hours))
        .await
        .context("query failure codes")?;
    let mut by_type: HashMap<String, Vec<FailureCodeCount>> = HashMap::new();
    for row in &rows {
        let task_type: String = row.try_get_by("task_type")?;
        by_type
            .entry(task_type)
            .or_default()
            .push(FailureCodeCount {
                code: row.try_get_by("code")?,
                count: row.try_get_by("count")?,
            });
    }
    Ok(by_type)
}

/// Build the statement; the backlog and failure-free queries take no window.
fn stmt(sql: &str, hours: i64) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, vec![hours.into()])
}

/// Read one `<prefix>_n/_avg/_p50/_p75/_p90/_p99` aggregate set.
pub(super) fn metric(row: &QueryResult, prefix: &str) -> anyhow::Result<MetricStats> {
    Ok(MetricStats {
        n: row.try_get_by(format!("{prefix}_n").as_str())?,
        avg: row.try_get_by(format!("{prefix}_avg").as_str())?,
        p50: row.try_get_by(format!("{prefix}_p50").as_str())?,
        p75: row.try_get_by(format!("{prefix}_p75").as_str())?,
        p90: row.try_get_by(format!("{prefix}_p90").as_str())?,
        p99: row.try_get_by(format!("{prefix}_p99").as_str())?,
    })
}
