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
            status: TaskRunStatus::from_i32(row.status_code)?,
            attempts: row.attempts,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_run_status_has_no_result_reference_fields() {
        let now = DateTime::parse_from_rfc3339("2026-10-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let info = TaskRunInfo {
            logical_id: "u7-illuminate-capture42".to_string(),
            run_id: "task-run-id".to_string(),
            run_number: 1,
            task_type: "illuminate".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 42,
            status: TaskRunStatus::CompleteSuccess,
            attempts: 1,
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_value(info).unwrap();

        assert_eq!(json["run_id"], "task-run-id");
        assert!(json.get("result_type").is_none());
        assert!(json.get("result_ref").is_none());
    }
}
