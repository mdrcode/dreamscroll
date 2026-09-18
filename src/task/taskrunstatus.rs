use anyhow::anyhow;

/// Typed status values which track a Run of a background Task.
///
/// Submission initially creates `Queued`. If the queue rejects the task before
/// a worker can receive it, the run becomes `SubmissionFailed`. Otherwise it
/// moves `Queued` → `InProgress` → one of the execution outcomes. A failure
/// with retry budget left becomes `ErrorWillRetry` (another attempt is coming);
/// once the budget is spent it becomes `CompleteFailure`.
///
/// The DB stores only the integer discriminant (`as_i32`); mapping between the
/// integer and this strongly-typed enum is owned here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskRunStatus {
    SubmissionFailed = 0,
    Queued = 1,
    InProgress = 2,
    ErrorWillRetry = 3,
    CompleteSuccess = 4,
    CompleteFailure = 5,
}

impl TaskRunStatus {
    /// The integer persisted in the `task_run_status.status_code` column.
    pub fn as_i32(&self) -> i32 {
        match self {
            TaskRunStatus::SubmissionFailed => 0,
            TaskRunStatus::Queued => 1,
            TaskRunStatus::InProgress => 2,
            TaskRunStatus::ErrorWillRetry => 3,
            TaskRunStatus::CompleteSuccess => 4,
            TaskRunStatus::CompleteFailure => 5,
        }
    }

    /// Reconstruct a `TaskRunStatus` from the integer stored in the DB.
    pub fn from_i32(v: i32) -> Result<Self, anyhow::Error> {
        match v {
            0 => Ok(TaskRunStatus::SubmissionFailed),
            1 => Ok(TaskRunStatus::Queued),
            2 => Ok(TaskRunStatus::InProgress),
            3 => Ok(TaskRunStatus::ErrorWillRetry),
            4 => Ok(TaskRunStatus::CompleteSuccess),
            5 => Ok(TaskRunStatus::CompleteFailure),
            other => Err(anyhow!("unknown task status integer: {other}")),
        }
    }

    /// True when a worker may still act on this run.
    ///
    /// This is the duplicate-submission predicate: while a run is in flight,
    /// submitting the same envelope again is refused. `CompleteSuccess` and
    /// `CompleteFailure` are terminal, so a rerun is permitted for those.
    pub fn is_in_flight(&self) -> bool {
        matches!(
            self,
            TaskRunStatus::Queued | TaskRunStatus::InProgress | TaskRunStatus::ErrorWillRetry
        )
    }
}

impl std::fmt::Display for TaskRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TaskRunStatus::SubmissionFailed => "SubmissionFailed",
            TaskRunStatus::Queued => "Queued",
            TaskRunStatus::InProgress => "InProgress",
            TaskRunStatus::ErrorWillRetry => "ErrorWillRetry",
            TaskRunStatus::CompleteSuccess => "CompleteSuccess",
            TaskRunStatus::CompleteFailure => "CompleteFailure",
        };
        f.write_str(format!("{}({})", name, self.as_i32()).as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i32_round_trip_is_stable() {
        // Discriminants are persisted; changing them silently would corrupt
        // every existing row.
        let all = [
            (TaskRunStatus::SubmissionFailed, 0),
            (TaskRunStatus::Queued, 1),
            (TaskRunStatus::InProgress, 2),
            (TaskRunStatus::ErrorWillRetry, 3),
            (TaskRunStatus::CompleteSuccess, 4),
            (TaskRunStatus::CompleteFailure, 5),
        ];

        for (status, code) in all {
            assert_eq!(status.as_i32(), code);
            assert_eq!(TaskRunStatus::from_i32(code).unwrap(), status);
        }
    }

    #[test]
    fn from_i32_rejects_unknown_values() {
        assert!(TaskRunStatus::from_i32(99).is_err());
        assert!(TaskRunStatus::from_i32(-1).is_err());
    }

    #[test]
    fn in_flight_means_a_worker_may_still_act() {
        assert!(TaskRunStatus::Queued.is_in_flight());
        assert!(TaskRunStatus::InProgress.is_in_flight());
        assert!(TaskRunStatus::ErrorWillRetry.is_in_flight());

        assert!(!TaskRunStatus::SubmissionFailed.is_in_flight());
        assert!(!TaskRunStatus::CompleteSuccess.is_in_flight());
        assert!(!TaskRunStatus::CompleteFailure.is_in_flight());
    }
}
