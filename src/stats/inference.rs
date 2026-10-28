//! Windowed inference duration and token stats.
//!
//! `inference_metas` rows are written only after the provider call returns, so
//! failed calls (including request timeouts) contribute no duration or token
//! cost here. Their rate and latency live in the queue stats instead.

use anyhow::Context;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde::{Deserialize, Serialize};

use crate::database;

use super::MetricStats;
use super::queue::metric;

/// One `(task_type, model, backend)` group within a window.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InferenceStats {
    pub task_type: String,
    pub model: String,
    pub backend: String,
    pub duration: MetricStats,
    pub tokens_in: MetricStats,
    pub tokens_in_cached: MetricStats,
    pub tokens_out: MetricStats,
    pub tokens_thought: MetricStats,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InferenceWindow {
    pub hours: i64,
    pub inferences: Vec<InferenceStats>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InferenceStatsResponse {
    pub windows: Vec<InferenceWindow>,
}

const INFERENCE_SQL: &str = r#"
SELECT
  COALESCE(task_type, 'unknown') AS task_type,
  model,
  backend,
  COUNT(*)::bigint AS dur_n,
  AVG(duration_ms)::double precision AS dur_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY duration_ms) AS dur_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY duration_ms) AS dur_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY duration_ms) AS dur_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY duration_ms) AS dur_p99,
  COUNT(tokens_in)::bigint AS tin_n,
  AVG(tokens_in)::double precision AS tin_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY tokens_in) AS tin_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY tokens_in) AS tin_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY tokens_in) AS tin_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY tokens_in) AS tin_p99,
  COUNT(tokens_in_cached)::bigint AS tic_n,
  AVG(tokens_in_cached)::double precision AS tic_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY tokens_in_cached) AS tic_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY tokens_in_cached) AS tic_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY tokens_in_cached) AS tic_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY tokens_in_cached) AS tic_p99,
  COUNT(tokens_out)::bigint AS tout_n,
  AVG(tokens_out)::double precision AS tout_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY tokens_out) AS tout_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY tokens_out) AS tout_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY tokens_out) AS tout_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY tokens_out) AS tout_p99,
  COUNT(tokens_thought)::bigint AS tth_n,
  AVG(tokens_thought)::double precision AS tth_avg,
  PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY tokens_thought) AS tth_p50,
  PERCENTILE_CONT(0.75) WITHIN GROUP (ORDER BY tokens_thought) AS tth_p75,
  PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY tokens_thought) AS tth_p90,
  PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY tokens_thought) AS tth_p99
FROM inference_metas
WHERE created_at >= now() - make_interval(hours => $1::int)
GROUP BY COALESCE(task_type, 'unknown'), model, backend
ORDER BY task_type, model, backend
"#;

pub async fn inference_stats(
    db: &database::DbHandle,
    windows: &[i64],
) -> anyhow::Result<InferenceStatsResponse> {
    let mut window_stats = Vec::with_capacity(windows.len());
    for &hours in windows {
        let rows = db
            .conn
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                INFERENCE_SQL,
                vec![hours.into()],
            ))
            .await
            .context("query inference stats")?;

        let inferences = rows
            .iter()
            .map(|row| {
                Ok(InferenceStats {
                    task_type: row.try_get_by("task_type")?,
                    model: row.try_get_by("model")?,
                    backend: row.try_get_by("backend")?,
                    duration: metric(row, "dur")?,
                    tokens_in: metric(row, "tin")?,
                    tokens_in_cached: metric(row, "tic")?,
                    tokens_out: metric(row, "tout")?,
                    tokens_thought: metric(row, "tth")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        window_stats.push(InferenceWindow { hours, inferences });
    }

    Ok(InferenceStatsResponse {
        windows: window_stats,
    })
}
