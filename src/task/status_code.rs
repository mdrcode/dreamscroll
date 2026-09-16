use anyhow::anyhow;

/// Typed status values for a background task.
///
/// The DB stores only the integer discriminant (`as_i32`); mapping between the
/// integer and this strongly-typed enum is owned here, in the task module.
///
/// `ErrorWillRetry` and `ErrorExhausted` are computed outcomes, not intrinsic
/// properties of an error: the same underlying failure is `ErrorWillRetry`
/// while the app still has retry budget, and `ErrorExhausted` once it is spent.
/// Only `Completed` is *complete*; everything else is incomplete (see
/// `is_incomplete`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCode {
    Queued = 0,
    InProgress = 1,
    Completed = 2,
    ErrorWillRetry = 3,
    ErrorExhausted = 4,
}

impl StatusCode {
    /// Every variant, so the incomplete predicate has a single source of truth
    /// to derive from.
    pub const ALL: [StatusCode; 5] = [
        StatusCode::Queued,
        StatusCode::InProgress,
        StatusCode::Completed,
        StatusCode::ErrorWillRetry,
        StatusCode::ErrorExhausted,
    ];

    /// The integer value persisted in the `task_status.status` column.
    pub fn as_i32(&self) -> i32 {
        match self {
            StatusCode::Queued => 0,
            StatusCode::InProgress => 1,
            StatusCode::Completed => 2,
            StatusCode::ErrorWillRetry => 3,
            StatusCode::ErrorExhausted => 4,
        }
    }

    /// Reconstruct a `Status` from the integer value stored in the DB.
    pub fn from_i32(v: i32) -> Result<Self, anyhow::Error> {
        match v {
            0 => Ok(StatusCode::Queued),
            1 => Ok(StatusCode::InProgress),
            2 => Ok(StatusCode::Completed),
            3 => Ok(StatusCode::ErrorWillRetry),
            4 => Ok(StatusCode::ErrorExhausted),
            other => Err(anyhow!("unknown task status integer: {other}")),
        }
    }

    /// True when the task has *not* completed successfully.
    ///
    /// "Incomplete" deliberately includes `ErrorExhausted`: the work never
    /// succeeded, so the user still wants to see it (and may want to retry it).
    /// Only `Completed` is complete.
    pub fn is_incomplete(&self) -> bool {
        !matches!(self, StatusCode::Completed)
    }

    /// Integer discriminants of every incomplete status, for SQL predicates.
    ///
    /// Derived from `is_incomplete` so the predicate and the status set can
    /// never drift apart.
    pub fn incomplete_codes() -> Vec<i32> {
        Self::ALL
            .iter()
            .filter(|status| status.is_incomplete())
            .map(StatusCode::as_i32)
            .collect()
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            StatusCode::Queued => "Queued",
            StatusCode::InProgress => "InProgress",
            StatusCode::Completed => "Completed",
            StatusCode::ErrorWillRetry => "ErrorWillRetry",
            StatusCode::ErrorExhausted => "ErrorExhausted",
        };
        f.write_str(format!("{}({})", name, self.as_i32()).as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_codes_excludes_only_completed() {
        let codes = StatusCode::incomplete_codes();

        assert_eq!(
            codes,
            vec![
                StatusCode::Queued.as_i32(),
                StatusCode::InProgress.as_i32(),
                StatusCode::ErrorWillRetry.as_i32(),
                StatusCode::ErrorExhausted.as_i32(),
            ]
        );

        assert!(
            !codes.contains(&StatusCode::Completed.as_i32()),
            "Completed is the only complete status and must not be queryable"
        );
        assert!(
            codes.contains(&StatusCode::ErrorExhausted.as_i32()),
            "ErrorExhausted is incomplete: the work never succeeded"
        );
    }

    #[test]
    fn i32_round_trip_is_stable() {
        // Discriminants are persisted; changing them silently would corrupt
        // every existing row.
        assert_eq!(StatusCode::Queued.as_i32(), 0);
        assert_eq!(StatusCode::InProgress.as_i32(), 1);
        assert_eq!(StatusCode::Completed.as_i32(), 2);
        assert_eq!(StatusCode::ErrorWillRetry.as_i32(), 3);
        assert_eq!(StatusCode::ErrorExhausted.as_i32(), 4);

        for status in StatusCode::ALL {
            assert_eq!(StatusCode::from_i32(status.as_i32()).unwrap(), status);
        }
    }
}
