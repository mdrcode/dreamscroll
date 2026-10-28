pub mod failures;
pub mod inference;
pub mod queue;

pub use failures::{QueueFailure, QueueFailuresResponse, queue_failures};
pub use inference::{InferenceStats, InferenceStatsResponse, InferenceWindow, inference_stats};
pub use queue::{QueueBacklogRow, QueueStats, QueueStatsResponse, QueueWindow, queue_stats};

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod db_tests;

/// Default reporting window when the caller does not name one.
pub const DEFAULT_WINDOW_HOURS: i64 = 24;
/// Guardrail: one calendar year, the widest window worth querying in one shot.
pub const MAX_WINDOW_HOURS: i64 = 24 * 365;

/// Compact distribution summary. Every field is `None`/zero when `n == 0`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MetricStats {
    pub n: i64,
    pub avg: Option<f64>,
    pub p50: Option<f64>,
    pub p75: Option<f64>,
    pub p90: Option<f64>,
    pub p99: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FailureCodeCount {
    /// `task_runs.last_error_code` verbatim; `None` when a failure recorded no code.
    pub code: Option<i32>,
    pub count: i64,
}

/// Parse a comma-separated list of window widths in hours, e.g. `1,24,168`.
/// Empty input yields `[DEFAULT_WINDOW_HOURS]`; duplicates are collapsed in order.
pub fn parse_windows(raw: Option<&str>) -> anyhow::Result<Vec<i64>> {
    let mut windows = Vec::new();
    for part in raw.unwrap_or("").split(',').map(str::trim) {
        if part.is_empty() {
            continue;
        }
        let hours: i64 = part
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid window hours: {part}"))?;
        anyhow::ensure!(
            (1..=MAX_WINDOW_HOURS).contains(&hours),
            "window hours must be between 1 and {MAX_WINDOW_HOURS}: {hours}"
        );
        if !windows.contains(&hours) {
            windows.push(hours);
        }
    }
    if windows.is_empty() {
        windows.push(DEFAULT_WINDOW_HOURS);
    }
    Ok(windows)
}

/// Parse a single window width in hours, defaulting to [`DEFAULT_WINDOW_HOURS`].
pub fn parse_window(raw: Option<&str>) -> anyhow::Result<i64> {
    let windows = parse_windows(raw)?;
    anyhow::ensure!(
        windows.len() == 1,
        "expected a single window, e.g. --hours 24 (got {windows:?})"
    );
    Ok(windows[0])
}

/// Cloud Tasks queue name for a task type. Types are snake_case; queues are
/// hyphenated (`search_index` → `search-index`).
pub fn queue_label(task_type: &str) -> String {
    task_type.replace('_', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_default_and_dedupe() {
        assert_eq!(parse_windows(None).unwrap(), vec![24]);
        assert_eq!(parse_windows(Some("")).unwrap(), vec![24]);
        assert_eq!(
            parse_windows(Some("1,24,168,720,24")).unwrap(),
            vec![1, 24, 168, 720]
        );
    }

    #[test]
    fn windows_reject_out_of_range() {
        assert!(parse_windows(Some("0")).is_err());
        assert!(parse_windows(Some("8761")).is_err());
        assert!(parse_windows(Some("24h")).is_err());
    }

    #[test]
    fn queue_labels_use_cloud_tasks_naming() {
        assert_eq!(queue_label("search_index"), "search-index");
        assert_eq!(queue_label("illuminate"), "illuminate");
    }
}
