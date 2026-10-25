use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model;
use crate::task::{TaskRunStatus, timing::TaskTimingEstimate};

pub const CURRENT_SCHEMA_VERSION: u8 = 4;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEventKind {
    TaskStatus,
}

/// Base container for all server events streamed to clients via SSE
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerEventEnvelope<E> {
    pub schema_version: u8,
    pub event_type: ServerEventKind,
    pub timestamp: DateTime<Utc>,
    pub entity_type: String,
    pub entity_id: i32,
    /// Owner of the entity this event is about; it is also the stream recipient.
    /// Task-status events use the target owner, never the submitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_owner_id: Option<i32>,
    pub payload: E,
}

impl<E> ServerEventEnvelope<E> {
    pub fn new(
        event_type: ServerEventKind,
        timestamp: DateTime<Utc>,
        entity_type: impl Into<String>,
        entity_id: i32,
        payload: E,
    ) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            event_type,
            timestamp,
            entity_type: entity_type.into(),
            entity_id,
            entity_owner_id: None,
            payload,
        }
    }
}

/// The concrete announcement streamed for a task status change.
pub type TaskStatusUpdate = ServerEventEnvelope<TaskStatusPayload>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskStatusPayload {
    pub task_type: String,
    pub status: TaskRunStatus,
    pub logical_id: String,
    pub run_id: String,
    pub run_number: i32,
    pub attempts: i32,
    /// When the most recent attempt entered `InProgress`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_started_at: Option<DateTime<Utc>>,
    /// Client-facing processing estimate carried by the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_duration_ms_p50: Option<i64>,
}

impl ServerEventEnvelope<TaskStatusPayload> {
    pub(crate) fn from_row(
        row: &model::task_run::Model,
        estimate: Option<&TaskTimingEstimate>,
    ) -> Option<Self> {
        let status = TaskRunStatus::from_i32(row.status_code).ok()?;
        let mut event = ServerEventEnvelope::new(
            ServerEventKind::TaskStatus,
            row.updated_at,
            row.entity_type.clone(),
            row.entity_id,
            TaskStatusPayload {
                task_type: row.task_type.clone(),
                status,
                logical_id: row.logical_id.clone(),
                run_id: row.run_id.clone(),
                run_number: row.run_number,
                attempts: row.attempts,
                processing_started_at: row.processing_started_at,
                estimated_duration_ms_p50: estimate
                    .and_then(TaskTimingEstimate::client_processing_duration_ms),
            },
        );
        event.entity_owner_id = Some(row.entity_owner_id);
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-21T18:42:10Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn task_status_update_has_stable_wire_shape() {
        let update = TaskStatusUpdate::new(
            ServerEventKind::TaskStatus,
            timestamp(),
            "capture",
            42,
            TaskStatusPayload {
                task_type: "illuminate".to_string(),
                status: TaskRunStatus::CompleteSuccess,
                logical_id: "u1-illuminate-capture42".to_string(),
                run_id: "b91a7c4f-7e8a-4bf8-9a76-c81e258ec113".to_string(),
                run_number: 3,
                attempts: 1,
                processing_started_at: None,
                estimated_duration_ms_p50: None,
            },
        );

        assert_eq!(
            serde_json::to_string(&update).unwrap(),
            r#"{"schema_version":4,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"task_type":"illuminate","status":{"name":"complete_success","discriminant":4},"logical_id":"u1-illuminate-capture42","run_id":"b91a7c4f-7e8a-4bf8-9a76-c81e258ec113","run_number":3,"attempts":1}}"#,
        );
    }

    #[test]
    fn task_status_constructor_sets_schema_and_routing_fields() {
        let mut row = model::task_run::task_run_model();
        row.entity_owner_id = 8;
        row.logical_id = "u8-test-capture91".to_string();
        row.run_number = 4;
        row.entity_id = 91;
        row.status_code = TaskRunStatus::CompleteSuccess.as_i32();
        row.attempts = 2;
        row.created_at = timestamp();
        row.updated_at = timestamp();

        let update = TaskStatusUpdate::from_row(&row, None).unwrap();

        assert_eq!(update.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(update.event_type, ServerEventKind::TaskStatus);
        assert_eq!(update.entity_type, "capture");
        assert_eq!(update.entity_id, 91);
        assert_eq!(update.payload.task_type, "illuminate");
        assert_eq!(update.payload.status, TaskRunStatus::CompleteSuccess);
        assert_eq!(update.payload.attempts, 2);
        assert_eq!(update.payload.run_number, 4);
        assert_eq!(update.entity_owner_id, Some(8));
    }
}
