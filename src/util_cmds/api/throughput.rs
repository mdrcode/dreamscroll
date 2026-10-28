use argh::FromArgs;

use crate::stats;

use super::*;

#[derive(FromArgs)]
#[argh(subcommand, name = "queue")]
#[argh(description = "Inspect queue timing and backlog via the admin API")]
pub struct QueueArgs {
    #[argh(subcommand)]
    command: QueueCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum QueueCommand {
    Throughput(QueueThroughputArgs),
    Failures(FailuresArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "throughput")]
#[argh(description = "Queued and in-process timing percentiles, split by success/failure")]
struct QueueThroughputArgs {
    #[argh(option)]
    #[argh(description = "window widths in hours, comma separated (default: 24)")]
    hours: Option<String>,

    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "failures")]
#[argh(description = "List recent queue failures with their error messages")]
struct FailuresArgs {
    #[argh(option)]
    #[argh(description = "window width in hours (default: 24)")]
    hours: Option<String>,

    #[argh(option)]
    #[argh(description = "maximum failures to list (default: 20, max 200)")]
    limit: Option<i64>,

    #[argh(switch)]
    #[argh(description = "also include runs waiting to retry (ErrorWillRetry)")]
    include_retrying: bool,

    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

#[derive(FromArgs)]
#[argh(subcommand, name = "inference")]
#[argh(description = "Inspect inference duration and token stats via the admin API")]
pub struct InferenceArgs {
    #[argh(subcommand)]
    command: InferenceCommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum InferenceCommand {
    Throughput(InferenceThroughputArgs),
}

#[derive(FromArgs)]
#[argh(subcommand, name = "throughput")]
#[argh(description = "Inference duration and token percentiles per task type and model")]
struct InferenceThroughputArgs {
    #[argh(option)]
    #[argh(description = "window widths in hours, comma separated (default: 24)")]
    hours: Option<String>,

    #[argh(switch)]
    #[argh(description = "print the raw JSON response instead of the report")]
    raw_json: bool,
}

pub async fn run_queue(state: ApiCmdState, args: QueueArgs) -> anyhow::Result<()> {
    match args.command {
        QueueCommand::Throughput(args) => {
            let response = state
                .rest_client
                .admin_queue_stats(args.hours.as_deref())
                .await?;
            if args.raw_json {
                print_json(&response)?;
            } else {
                print!("{}", render_queue(&response));
            }
        }
        QueueCommand::Failures(args) => {
            let response = state
                .rest_client
                .admin_queue_failures(args.hours.as_deref(), args.limit, args.include_retrying)
                .await?;
            if args.raw_json {
                print_json(&response)?;
            } else {
                print!("{}", render_failures(&response));
            }
        }
    }
    Ok(())
}

pub async fn run_inference(state: ApiCmdState, args: InferenceArgs) -> anyhow::Result<()> {
    let (hours, raw_json) = match args.command {
        InferenceCommand::Throughput(args) => (args.hours, args.raw_json),
    };
    let response = state
        .rest_client
        .admin_inference_stats(hours.as_deref())
        .await?;
    if raw_json {
        print_json(&response)?;
    } else {
        print!("{}", render_inference(&response));
    }
    Ok(())
}

const QUEUE_STAT_HEADERS: [&str; 8] = ["n", "Queue", "State", "avg", "p50", "p75", "p90", "p99"];
const QUEUE_STAT_ALIGN: [bool; 8] = [true, false, false, true, true, true, true, true];
const PHASE_STAT_HEADERS: [&str; 8] = ["n", "Phase", "Outcome", "avg", "p50", "p75", "p90", "p99"];
const PHASE_STAT_ALIGN: [bool; 8] = [true, false, false, true, true, true, true, true];
const FAILURE_HEADERS: [&str; 7] = [
    "when", "queue", "entity", "att", "code", "duration", "message",
];
const FAILURE_ALIGN: [bool; 7] = [true, false, false, true, true, true, false];
const METRIC_STAT_HEADERS: [&str; 7] = ["n", "Metric", "avg", "p50", "p75", "p90", "p99"];
const METRIC_STAT_ALIGN: [bool; 7] = [true, false, true, true, true, true, true];

fn render_queue(response: &stats::QueueStatsResponse) -> String {
    let mut out = String::new();
    if response.snapshot_suppressed {
        out.push_str("Suppressing current snapshot (in flight) for TASK_BACKEND=local\n");
        out.push_str(
            "  (local queue drops in-memory tasks on restart, leaving stale Queued/InProgress rows)\n",
        );
    } else {
        out.push_str("Current snapshot (Queued + InProgress + ErrorWillRetry)\n");
        let mut rows = Vec::with_capacity(response.backlog.len() * 3);
        for row in &response.backlog {
            for (state, stats) in [
                ("queued", &row.queued),
                ("in_progress", &row.in_progress),
                ("error_will_retry", &row.error_will_retry),
            ] {
                if stats.n > 0 {
                    rows.push(stats_row(&[&row.queue, state], stats, fmt_ms));
                }
            }
        }
        if rows.is_empty() {
            out.push_str("  (none)\n");
        } else {
            push_table(&mut out, &QUEUE_STAT_HEADERS, &rows, &QUEUE_STAT_ALIGN);
        }
    }

    for window in &response.windows {
        out.push('\n');
        out.push_str(&format!("== Last {} ==\n", fmt_hours(window.hours)));
        if window.queues.is_empty() {
            out.push_str("  (no runs)\n");
            continue;
        }
        for queue in &window.queues {
            out.push('\n');
            out.push_str(&format!("{}  (queue: {})\n", queue.task_type, queue.queue));
            let mut rows = Vec::with_capacity(4);
            for (phase, outcome, stats) in [
                ("queued", "success", &queue.queued_success),
                ("queued", "failure", &queue.queued_failure),
                ("processing", "success", &queue.processing_success),
                ("processing", "failure", &queue.processing_failure),
            ] {
                if stats.n > 0 {
                    rows.push(stats_row(&[phase, outcome], stats, fmt_ms));
                }
            }
            if rows.is_empty() {
                out.push_str("  (no runs)\n");
            } else {
                push_table(&mut out, &PHASE_STAT_HEADERS, &rows, &PHASE_STAT_ALIGN);
            }
            if !queue.failure_codes.is_empty() {
                let codes = queue
                    .failure_codes
                    .iter()
                    .map(|entry| match entry.code {
                        Some(code) => format!("{code} x{}", entry.count),
                        None => format!("no-code x{}", entry.count),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("  failures: {codes}\n"));
            }
        }
    }

    out
}

fn render_inference(response: &stats::InferenceStatsResponse) -> String {
    let mut out = String::new();
    for window in &response.windows {
        out.push('\n');
        out.push_str(&format!("== Last {} ==\n", fmt_hours(window.hours)));
        if window.inferences.is_empty() {
            out.push_str("  (no inference records)\n");
            continue;
        }
        for inference in &window.inferences {
            out.push('\n');
            out.push_str(&format!(
                "{}  {}  {}\n",
                inference.task_type, inference.model, inference.backend
            ));
            let rows = [
                stats_row(&["duration"], &inference.duration, fmt_ms),
                stats_row(&["tokens_in"], &inference.tokens_in, fmt_qty),
                stats_row(&["tokens_in_cached"], &inference.tokens_in_cached, fmt_qty),
                stats_row(&["tokens_out"], &inference.tokens_out, fmt_qty),
                stats_row(&["tokens_thought"], &inference.tokens_thought, fmt_qty),
            ];
            push_table(&mut out, &METRIC_STAT_HEADERS, &rows, &METRIC_STAT_ALIGN);
        }
    }
    out
}

fn render_failures(response: &stats::QueueFailuresResponse) -> String {
    let label = if response.include_retrying {
        "failures + retrying"
    } else {
        "terminal failures"
    };
    let mut out = format!(
        "== Last {}, {} (n={}) ==\n",
        fmt_hours(response.hours),
        label,
        response.failures.len()
    );
    if response.failures.is_empty() {
        out.push_str("  (none)\n");
        return out;
    }

    let rows = response
        .failures
        .iter()
        .map(|failure| {
            vec![
                format!("{} ago", fmt_ms(Some(failure.age_ms as f64))),
                failure.queue.clone(),
                format!("{} {}", failure.entity_type, failure.entity_id),
                failure.attempts.to_string(),
                failure
                    .error_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "—".to_string()),
                failure
                    .duration_ms
                    .map(|ms| fmt_ms(Some(ms as f64)))
                    .unwrap_or_else(|| "—".to_string()),
                clip(failure.error_message.as_deref().unwrap_or("—"), 100),
            ]
        })
        .collect::<Vec<_>>();
    push_table(&mut out, &FAILURE_HEADERS, &rows, &FAILURE_ALIGN);
    out
}

/// First line, truncated to `max` chars with an ellipsis when cut.
fn clip(message: &str, max: usize) -> String {
    let line = message.lines().next().unwrap_or("");
    let mut chars = line.chars();
    let clipped: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

/// One table row: `n`, then the labels, then `avg, p50, p75, p90, p99`.
fn stats_row(
    labels: &[&str],
    stats: &stats::MetricStats,
    format: fn(Option<f64>) -> String,
) -> Vec<String> {
    let mut row = vec![stats.n.to_string()];
    row.extend(labels.iter().map(|label| label.to_string()));
    for value in [stats.avg, stats.p50, stats.p75, stats.p90, stats.p99] {
        row.push(format(value));
    }
    row
}

/// Append a column-aligned table. `right[i]` selects right alignment per column
/// so numeric columns line up regardless of where they sit.
fn push_table(out: &mut String, headers: &[&str], rows: &[Vec<String>], right: &[bool]) {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let header: Vec<String> = headers.iter().map(|header| header.to_string()).collect();
    out.push_str("  ");
    out.push_str(&table_line(&header, &widths, right));
    out.push('\n');
    for row in rows {
        out.push_str("  ");
        out.push_str(&table_line(row, &widths, right));
        out.push('\n');
    }
}

fn table_line(cells: &[String], widths: &[usize], right: &[bool]) -> String {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let width = widths[index];
            if right[index] {
                format!("{cell:>width$}")
            } else {
                format!("{cell:<width$}")
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
        .trim_end()
        .to_string()
}

fn fmt_ms(value: Option<f64>) -> String {
    match value {
        None => "—".to_string(),
        Some(ms) if ms < 1_000.0 => format!("{ms:.0}ms"),
        Some(ms) if ms < 120_000.0 => format!("{:.1}s", ms / 1_000.0),
        Some(ms) if ms < 90.0 * 60_000.0 => format!("{:.1}m", ms / 60_000.0),
        Some(ms) => format!("{:.1}h", ms / 3_600_000.0),
    }
}

fn fmt_qty(value: Option<f64>) -> String {
    match value {
        None => "—".to_string(),
        Some(qty) if qty >= 1_000.0 => format!("{:.1}k", qty / 1_000.0),
        Some(qty) => format!("{qty:.0}"),
    }
}

fn fmt_hours(hours: i64) -> String {
    if hours >= 24 && hours % 24 == 0 {
        format!("{hours}h ({}d)", hours / 24)
    } else {
        format!("{hours}h")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(n: i64, value: f64) -> stats::MetricStats {
        stats::MetricStats {
            n,
            avg: Some(value),
            p50: Some(value),
            p75: Some(value),
            p90: Some(value),
            p99: Some(value),
        }
    }

    fn sample_queue_response() -> stats::QueueStatsResponse {
        stats::QueueStatsResponse {
            snapshot_suppressed: false,
            backlog: vec![stats::QueueBacklogRow {
                task_type: "illuminate".into(),
                queue: "illuminate".into(),
                queued: metric(388, 2.7 * 3_600_000.0),
                in_progress: metric(1, 226_000.0),
                error_will_retry: metric(2, 45_000.0),
            }],
            windows: vec![stats::QueueWindow {
                hours: 24,
                queues: vec![stats::QueueStats {
                    task_type: "illuminate".into(),
                    queue: "illuminate".into(),
                    queued_success: metric(380, 12_400.0),
                    queued_failure: metric(12, 246_000.0),
                    processing_success: metric(368, 74_000.0),
                    processing_failure: metric(12, 300_000.0),
                    failure_codes: vec![stats::FailureCodeCount {
                        code: Some(504),
                        count: 9,
                    }],
                }],
            }],
        }
    }

    fn assert_block_aligned(lines: &[&str], header_index: usize, rows: usize) {
        let block = &lines[header_index..header_index + rows];
        let width = block[0].chars().count();
        assert!(
            block.iter().all(|line| line.chars().count() == width),
            "ragged table:\n{}",
            block.join("\n")
        );
    }

    #[test]
    fn renders_queue_report_as_aligned_tables() {
        let text = render_queue(&sample_queue_response());
        let lines: Vec<&str> = text.lines().collect();

        let backlog_header = lines
            .iter()
            .position(|line| line.trim_start().starts_with("n  Queue"))
            .expect("in-flight header");
        assert_block_aligned(&lines, backlog_header, 4);
        assert!(lines[backlog_header + 1].trim_start().starts_with("388"));

        let phase_header = lines
            .iter()
            .position(|line| line.trim_start().starts_with("n  Phase"))
            .expect("phase header");
        assert_block_aligned(&lines, phase_header, 5);

        assert!(text.contains("Current snapshot (Queued + InProgress + ErrorWillRetry)"));
        assert!(text.contains("failures: 504 x9"));
        assert!(text.contains("== Last 24h (1d) =="));
    }

    #[test]
    fn renders_failure_list() {
        let response = stats::QueueFailuresResponse {
            hours: 24,
            include_retrying: false,
            failures: vec![
                stats::QueueFailure {
                    queue: "illuminate".into(),
                    task_type: "illuminate".into(),
                    entity_type: "capture".into(),
                    entity_id: 1306,
                    run_id: "run-1".into(),
                    attempts: 3,
                    status: "complete_failure".into(),
                    error_code: Some(504),
                    error_message: Some("request terminated at 300s\nstack tail".into()),
                    duration_ms: Some(300_000),
                    age_ms: 744_000,
                },
                stats::QueueFailure {
                    queue: "illuminate".into(),
                    task_type: "illuminate".into(),
                    entity_type: "capture".into(),
                    entity_id: 1201,
                    run_id: "run-2".into(),
                    attempts: 1,
                    status: "complete_failure".into(),
                    error_code: None,
                    error_message: None,
                    duration_ms: None,
                    age_ms: 2_472_000,
                },
            ],
        };
        let text = render_failures(&response);
        assert!(text.contains("terminal failures (n=2)"));
        assert!(text.contains("capture 1306"));
        assert!(text.contains("request terminated at 300s"));
        assert!(!text.contains("stack tail"));
        let lines: Vec<&str> = text.lines().collect();
        let header = lines
            .iter()
            .position(|line| line.trim_start().starts_with("when"))
            .expect("failure header");
        let header_line = lines[header];
        let first = lines[header + 1];
        let second = lines[header + 2];
        // The message column is last and left-aligned, so compare its start
        // offset rather than total widths (trailing padding is trimmed).
        let message_byte = first.find("request terminated").expect("message cell");
        let message_column = first[..message_byte].chars().count();
        assert!(header_line.trim_end().ends_with("message"));
        // Compare character offsets: em dashes are multibyte, so byte offsets skew.
        let second_byte = second.rmatch_indices('—').next().expect("dash cell").0;
        let second_column = second[..second_byte].chars().count();
        assert_eq!(second_column, message_column);
    }

    #[test]
    fn renders_empty_failure_list() {
        let response = stats::QueueFailuresResponse {
            hours: 24,
            include_retrying: true,
            failures: Vec::new(),
        };
        let text = render_failures(&response);
        assert!(text.contains("failures + retrying (n=0)"));
        assert!(text.contains("(none)"));
    }

    #[test]
    fn suppresses_empty_buckets() {
        let response = stats::QueueStatsResponse {
            snapshot_suppressed: false,
            backlog: vec![stats::QueueBacklogRow {
                task_type: "illuminate".into(),
                queue: "illuminate".into(),
                queued: metric(1, 2_000.0),
                in_progress: stats::MetricStats::default(),
                error_will_retry: stats::MetricStats::default(),
            }],
            windows: vec![stats::QueueWindow {
                hours: 24,
                queues: vec![stats::QueueStats {
                    task_type: "illuminate".into(),
                    queue: "illuminate".into(),
                    queued_success: metric(1, 12_000.0),
                    queued_failure: stats::MetricStats::default(),
                    processing_success: stats::MetricStats::default(),
                    processing_failure: stats::MetricStats::default(),
                    failure_codes: Vec::new(),
                }],
            }],
        };
        let text = render_queue(&response);
        assert!(!text.contains("failure"));
        assert!(!text.contains("in_progress"));
        assert!(!text.contains("error_will_retry"));
        assert!(text.contains("queued  success"));
    }

    #[test]
    fn suppresses_snapshot_for_local_backend() {
        let mut response = sample_queue_response();
        response.snapshot_suppressed = true;
        let text = render_queue(&response);
        assert!(text.contains("Suppressing current snapshot (in flight) for TASK_BACKEND=local"));
        assert!(!text.contains("n  Queue"));
    }

    #[test]
    fn renders_inference_report_as_aligned_tables() {
        let response = stats::InferenceStatsResponse {
            windows: vec![stats::InferenceWindow {
                hours: 24,
                inferences: vec![stats::InferenceStats {
                    task_type: "illuminate".into(),
                    model: "gemini-3.8-flash".into(),
                    backend: "vertex".into(),
                    duration: metric(29, 74_000.0),
                    tokens_in: metric(29, 3_120.0),
                    tokens_in_cached: stats::MetricStats::default(),
                    tokens_out: metric(29, 820.0),
                    tokens_thought: stats::MetricStats::default(),
                }],
            }],
        };
        let text = render_inference(&response);
        let lines: Vec<&str> = text.lines().collect();
        let header = lines
            .iter()
            .position(|line| line.trim_start().starts_with("n  Metric"))
            .expect("metric header");
        assert_block_aligned(&lines, header, 6);
        assert!(text.contains("gemini-3.8-flash"));
    }

    #[test]
    fn formats_durations_and_empty_sets() {
        assert_eq!(fmt_ms(Some(240.0)), "240ms");
        assert_eq!(fmt_ms(Some(1_500.0)), "1.5s");
        assert_eq!(fmt_ms(Some(226_000.0)), "3.8m");
        assert_eq!(fmt_ms(Some(2.7 * 3_600_000.0)), "2.7h");
        assert_eq!(fmt_ms(None), "—");
        let empty = stats_row(
            &["queued", "success"],
            &stats::MetricStats::default(),
            fmt_ms,
        );
        assert_eq!(empty[0], "0");
        assert_eq!(empty[3], "—");
    }

    #[test]
    fn formats_token_quantities() {
        assert_eq!(fmt_qty(Some(812.0)), "812");
        assert_eq!(fmt_qty(Some(3_120.0)), "3.1k");
        assert_eq!(fmt_qty(None), "—");
    }

    #[test]
    fn formats_windows_in_days_when_whole() {
        assert_eq!(fmt_hours(6), "6h");
        assert_eq!(fmt_hours(24), "24h (1d)");
        assert_eq!(fmt_hours(168), "168h (7d)");
    }
}
