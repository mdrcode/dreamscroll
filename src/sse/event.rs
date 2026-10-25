use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model;
use crate::task::{TaskRunStatus, timing::TaskTimingEstimate};

pub const CURRENT_SCHEMA_VERSION: u8 = 4;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEventTypes {
    TaskStatus,
}

/// Base container for all server events streamed to clients via SSE
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerEvent<E> {
    pub schema_version: u8,
    pub event_type: ServerEventTypes,
    pub timestamp: DateTime<Utc>,
    pub entity_type: String,
    pub entity_id: i32,
    /// Recipient user ID for per-user routing; task-status events use the target owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i32>,
    pub payload: E,
}

// Concrete payload-enriched ServerEvent types
pub type TaskStatusEvent = ServerEvent<TaskStatusPayload>;

impl TaskStatusEvent {
    /// Whether this event is addressed to `user_id`.
    pub fn belongs_to(&self, user_id: i32) -> bool {
        self.user_id == Some(user_id)
    }
}

impl<E> ServerEvent<E>
where
    E: Clone + Serialize,
{
    /// Browser wire form: the notify payload minus the routing-only `user_id`.
    ///
    /// `user_id` stays in `Serialize` because that impl also feeds `pg_notify`,
    /// where the field routes an event to the right user on every instance.
    pub fn browser_json(&self) -> serde_json::Result<String> {
        let mut wire = self.clone();
        wire.user_id = None;
        serde_json::to_string(&wire)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskStatusPayload {
    pub task_type: String,
    pub status: TaskRunStatus,
    pub logical_id: String,
    pub run_id: String,
    pub run_number: i32,
    pub attempts: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_duration_ms_p50: Option<i64>,
}

impl<E> ServerEvent<E> {
    pub fn new(
        event_type: ServerEventTypes,
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
            user_id: None,
            payload,
        }
    }
}

impl ServerEvent<TaskStatusPayload> {
    pub(crate) fn from_row(
        row: &model::task_run::Model,
        estimate: Option<&TaskTimingEstimate>,
    ) -> Option<Self> {
        let status = TaskRunStatus::from_i32(row.status_code).ok()?;
        let mut event = ServerEvent::new(
            ServerEventTypes::TaskStatus,
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
        event.user_id = Some(row.entity_owner_id);
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
    fn belongs_to_matches_only_its_recipient() {
        let event = |user_id: i32| TaskStatusEvent {
            user_id: Some(user_id),
            ..TaskStatusEvent::new(
                ServerEventTypes::TaskStatus,
                timestamp(),
                "capture",
                42,
                TaskStatusPayload {
                    task_type: "illuminate".to_string(),
                    status: TaskRunStatus::InProgress,
                    logical_id: format!("u{user_id}-illuminate-capture42"),
                    run_id: "6f6844ec-d1ac-4f51-b4ae-57e92c646b8d".to_string(),
                    run_number: 1,
                    attempts: 1,
                    processing_started_at: None,
                    estimated_duration_ms_p50: None,
                },
            )
        };

        assert!(event(7).belongs_to(7));
        assert!(!event(8).belongs_to(7));
    }

    #[test]
    fn browser_json_omits_internal_user_id() {
        let update = TaskStatusEvent {
            user_id: Some(77),
            ..TaskStatusEvent::new(
                ServerEventTypes::TaskStatus,
                timestamp(),
                "capture",
                42,
                TaskStatusPayload {
                    task_type: "illuminate".to_string(),
                    status: TaskRunStatus::InProgress,
                    logical_id: "u77-illuminate-capture42".to_string(),
                    run_id: "720196b8-3dbf-4c63-9b78-c3f05787d831".to_string(),
                    run_number: 2,
                    attempts: 1,
                    processing_started_at: None,
                    estimated_duration_ms_p50: None,
                },
            )
        };

        let event_json = update.browser_json().unwrap();
        assert!(!event_json.contains("user_id"));
        assert!(event_json.contains("\"entity_id\":42"));
        assert!(event_json.contains("\"event_type\":\"task_status\""));
        assert!(
            serde_json::to_string(&update).unwrap().contains("user_id"),
            "the notify form keeps the routing field"
        );
    }

    #[test]
    fn task_status_update_has_stable_wire_shape() {
        let update = TaskStatusEvent::new(
            ServerEventTypes::TaskStatus,
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

        let update = TaskStatusEvent::from_row(&row, None).unwrap();

        assert_eq!(update.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(update.event_type, ServerEventTypes::TaskStatus);
        assert_eq!(update.entity_type, "capture");
        assert_eq!(update.entity_id, 91);
        assert_eq!(update.payload.task_type, "illuminate");
        assert_eq!(update.payload.status, TaskRunStatus::CompleteSuccess);
        assert_eq!(update.payload.attempts, 2);
        assert_eq!(update.payload.run_number, 4);
        assert_eq!(update.user_id, Some(8));
    }
}
