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
    /// Stable snake-case name used on the JSON wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskRunStatus::SubmissionFailed => "submission_failed",
            TaskRunStatus::Queued => "queued",
            TaskRunStatus::InProgress => "in_progress",
            TaskRunStatus::ErrorWillRetry => "error_will_retry",
            TaskRunStatus::CompleteSuccess => "complete_success",
            TaskRunStatus::CompleteFailure => "complete_failure",
        }
    }

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

#[derive(serde::Deserialize, serde::Serialize)]
struct TaskRunStatusWire {
    name: String,
    discriminant: i32,
}

impl serde::Serialize for TaskRunStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        TaskRunStatusWire {
            name: self.as_str().to_string(),
            discriminant: self.as_i32(),
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for TaskRunStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TaskRunStatusWire::deserialize(deserializer)?;
        let status = Self::from_i32(wire.discriminant).map_err(serde::de::Error::custom)?;

        if wire.name != status.as_str() {
            return Err(serde::de::Error::custom(format!(
                "task status name {:?} does not match discriminant {}",
                wire.name, wire.discriminant
            )));
        }

        Ok(status)
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

    #[test]
    fn display_includes_name_and_persisted_code() {
        assert_eq!(
            TaskRunStatus::SubmissionFailed.to_string(),
            "SubmissionFailed(0)"
        );
        assert_eq!(
            TaskRunStatus::CompleteFailure.to_string(),
            "CompleteFailure(5)"
        );
    }

    #[test]
    fn serde_uses_name_and_discriminant() {
        assert_eq!(
            serde_json::to_string(&TaskRunStatus::CompleteSuccess).unwrap(),
            r#"{"name":"complete_success","discriminant":4}"#
        );
        assert_eq!(
            serde_json::from_str::<TaskRunStatus>(
                r#"{"name":"error_will_retry","discriminant":3}"#
            )
            .unwrap(),
            TaskRunStatus::ErrorWillRetry
        );
    }

    #[test]
    fn serde_rejects_mismatched_name_and_discriminant() {
        assert!(
            serde_json::from_str::<TaskRunStatus>(r#"{"name":"queued","discriminant":4}"#).is_err()
        );
    }

    #[test]
    fn only_in_flight_statuses_block_duplicate_submission() {
        for status in [
            TaskRunStatus::Queued,
            TaskRunStatus::InProgress,
            TaskRunStatus::ErrorWillRetry,
        ] {
            assert!(status.is_in_flight(), "{status} should block duplicates");
        }

        for status in [
            TaskRunStatus::SubmissionFailed,
            TaskRunStatus::CompleteSuccess,
            TaskRunStatus::CompleteFailure,
        ] {
            assert!(!status.is_in_flight(), "{status} should allow a rerun");
        }
    }
}
