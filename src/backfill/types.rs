//! Shared backfill request/result types, not database models. The engine and
//! admin API use these directly; REST handlers and clients serialize them as JSON.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{api, task};

/// Request one bounded batch of submissions, not execution to completion.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillPumpRequest {
    pub backfill_id: String,
    /// Maximum candidates attempted (not successful enqueues); positive, defaults to 100.
    pub limit: Option<u64>,
    /// Pump despite a recorded dry streak. Required after a pump found nothing
    /// to submit; a forced pump that finds work clears the dry state.
    #[serde(default)]
    pub force: bool,
}

/// A submission error does not imply no TaskRun was persisted for this target.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillSubmissionError {
    pub entity_id: i32,
    pub error: String,
}

/// Submission outcomes for this pump only, not cumulative backfill progress.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillPumpResponse {
    pub backfill_id: String,
    /// Candidates selected and attempted, including skips and errors.
    pub selected_count: usize,
    pub enqueued_count: usize,
    /// Already recorded for this backfill by the time submission was attempted
    /// (e.g. another pump won the race after selection).
    pub already_submitted_count: usize,
    /// Conflicting work is in flight; no new run was created, so later pumps can retry.
    pub refused_in_flight_count: usize,
    pub errors: Vec<BackfillSubmissionError>,
    /// Recorded dry streak start after this pump. `None` means this pump found
    /// candidates, so pumping stays open.
    pub dry_since: Option<DateTime<Utc>>,
    /// Database time at which this pump's outcome was recorded.
    pub pumped_at: DateTime<Utc>,
}

/// Number of recorded TaskRuns in one task-type/status group, not retry attempts.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillStatusCount {
    pub task_type: String,
    pub status: task::TaskRunStatus,
    pub count: u64,
}

/// Current aggregate status across all runs tagged with this backfill ID,
/// including descendants, plus what pumping has observed so far.
///
/// An empty or successful snapshot is not a completion guarantee: target
/// membership is live and descendants may still be submitted.
#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillStatusResponse {
    pub backfill_id: String,
    /// Last pump that actually ran; a refused pump writes nothing. Absent until
    /// the first successful pump.
    pub last_pumped_at: Option<DateTime<Utc>>,
    /// Recorded dry streak start: while set, pumps are refused unless forced,
    /// and a forced pump that finds work clears it. A submission-stream
    /// observation, not a completion claim.
    pub dry_since: Option<DateTime<Utc>>,
    /// Recorded TaskRuns by task type and status, including descendants: what
    /// exists so far, never a frozen total.
    pub counts: Vec<BackfillStatusCount>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackfillFailuresResponse {
    pub backfill_id: String,
    pub failures: Vec<api::TaskRunInfo>,
}
