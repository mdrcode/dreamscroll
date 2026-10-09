use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::task::TaskRunStatus;

/// User-visible status of one task run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskRunInfo {
    pub logical_id: String,
    pub run_id: String,
    pub run_number: i32,
    pub task_type: String,
    pub entity_type: String,
    pub entity_id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_entity_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_entity_id: Option<String>,
    pub status: TaskRunStatus,
    pub attempts: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<crate::model::task_run::Model> for TaskRunInfo {
    type Error = anyhow::Error;

    fn try_from(row: crate::model::task_run::Model) -> Result<Self, Self::Error> {
        Ok(Self {
            logical_id: row.logical_id,
            run_id: row.run_id,
            run_number: row.run_number,
            task_type: row.task_type,
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            result_entity_type: row.result_entity_type,
            result_entity_id: row.result_entity_id,
            status: TaskRunStatus::from_i32(row.status_code)?,
            attempts: row.attempts,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}
