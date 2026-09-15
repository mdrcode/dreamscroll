use anyhow::anyhow;

/// Typed status values for a background task.
///
/// The DB stores only the integer discriminant (`as_i32`); mapping between the
/// integer and this strongly-typed enum is owned here, in the task module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCode {
    Queued = 0,
    InProgress = 1,
    Completed = 2,
    Error = 3,
    ErrorFinal = 4,
}

impl StatusCode {
    /// The integer value persisted in the `task_status.status` column.
    pub fn as_i32(&self) -> i32 {
        match self {
            StatusCode::Queued => 0,
            StatusCode::InProgress => 1,
            StatusCode::Completed => 2,
            StatusCode::Error => 3,
            StatusCode::ErrorFinal => 4,
        }
    }

    /// Reconstruct a `Status` from the integer value stored in the DB.
    pub fn from_i32(v: i32) -> Result<Self, anyhow::Error> {
        match v {
            0 => Ok(StatusCode::Queued),
            1 => Ok(StatusCode::InProgress),
            2 => Ok(StatusCode::Completed),
            3 => Ok(StatusCode::Error),
            4 => Ok(StatusCode::ErrorFinal),
            other => Err(anyhow!("unknown task status integer: {other}")),
        }
    }
}

impl std::fmt::Display for StatusCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            StatusCode::Queued => "Queued",
            StatusCode::InProgress => "InProgress",
            StatusCode::Completed => "Completed",
            StatusCode::Error => "Error",
            StatusCode::ErrorFinal => "ErrorFinal",
        };
        f.write_str(format!("{}({})", name, self.as_i32()).as_str())
    }
}
