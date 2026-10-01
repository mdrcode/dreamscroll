use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::task::TaskRunStatus;

/// Stable public identity returned when a task run is submitted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskRunIdentity {
    pub envelope_id: String,
    pub run: i32,
}

/// User-visible status of one task run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskRunInfo {
    pub envelope_id: String,
    pub run: i32,
    pub task_type: String,
    pub entity_type: String,
    pub entity_id: i32,
    pub status: TaskRunStatus,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<crate::model::task_run_status::Model> for TaskRunInfo {
    type Error = anyhow::Error;

    fn try_from(row: crate::model::task_run_status::Model) -> Result<Self, Self::Error> {
        Ok(Self {
            envelope_id: row.envelope_id,
            run: row.run,
            task_type: row.task_type,
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            status: TaskRunStatus::from_i32(row.status_code)?,
            attempts: row.attempts,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}