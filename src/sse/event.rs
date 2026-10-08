use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model;
use crate::task::{Task, TaskEnvelope, TaskRunStatus, timing::TaskTimingEstimate};

pub const CURRENT_SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEventTypes {
    Availability,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i32>,
    pub payload: E,
}

// Concrete payload-enriched ServerEvent types
pub type AvailabilityEvent = ServerEvent<AvailabilityPayload>;
pub type TaskStatusEvent = ServerEvent<TaskStatusPayload>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    Available,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AvailabilityPayload {
    pub operation: AvailabilityState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskStatusPayload {
    pub task_type: String,
    pub status: TaskRunStatus,
    pub run: i32,
    pub attempts: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_entity_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_entity_id: Option<String>,
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
    pub(crate) fn from_envelope<T: Task>(
        envelope: &TaskEnvelope<T>,
        row: &model::task_run_status::Model,
        estimate: Option<&TaskTimingEstimate>,
    ) -> Self {
        let mut event = Self::from_row(row, estimate).expect("validated task status row");
        event.payload.task_type = T::task_type().to_string();
        event.entity_type = T::entity_type().to_string();
        event.entity_id = envelope.task.entity_id();
        event.user_id = Some(envelope.user_id);
        event.payload.run = envelope.run;
        event.payload.estimated_duration_ms_p50 =
            estimate.and_then(TaskTimingEstimate::client_processing_duration_ms);
        event
    }

    pub(crate) fn from_row(
        row: &model::task_run_status::Model,
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
                attempts: row.attempts,
                result_entity_type: row.result_entity_type.clone(),
                result_entity_id: row.result_entity_id.clone(),
                run: row.run,
                processing_started_at: row.processing_started_at,
                estimated_duration_ms_p50: estimate
                    .and_then(TaskTimingEstimate::client_processing_duration_ms),
            },
        );
        event.user_id = Some(row.user_id);
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
        let update = TaskStatusEvent::new(
            ServerEventTypes::TaskStatus,
            timestamp(),
            "capture",
            42,
            TaskStatusPayload {
                task_type: "illuminate".to_string(),
                run: 3,
                status: TaskRunStatus::CompleteSuccess,
                attempts: 1,
                result_entity_type: None,
                result_entity_id: None,
                processing_started_at: None,
                estimated_duration_ms_p50: None,
            },
        );

        assert_eq!(
            serde_json::to_string(&update).unwrap(),
            r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"task_type":"illuminate","status":{"name":"complete_success","discriminant":4},"run":3,"attempts":1}}"#
        );
    }

    #[test]
    fn task_status_constructor_sets_schema_and_routing_fields() {
        let update = TaskStatusEvent::from_row(
            &model::task_run_status::Model {
                id: 1,
                user_id: 8,
                envelope_id: "u8-test-capture91".to_string(),
                run: 4,
                task_type: "illuminate".to_string(),
                entity_type: "capture".to_string(),
                entity_id: 91,
                task_payload: None,
                result_entity_type: Some("inference".to_string()),
                result_entity_id: Some("8a0d329d-72ca-4fd5-bd8d-2302f762f37d".to_string()),
                status_code: TaskRunStatus::CompleteSuccess.as_i32(),
                attempts: 2,
                created_at: timestamp(),
                processing_started_at: None,
                last_error_duration_ms: None,
                success_duration_ms: None,
                updated_at: timestamp(),
            },
            None,
        )
        .unwrap();

        assert_eq!(update.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(update.event_type, ServerEventTypes::TaskStatus);
        assert_eq!(update.entity_type, "capture");
        assert_eq!(update.entity_id, 91);
        assert_eq!(update.payload.task_type, "illuminate");
        assert_eq!(update.payload.status, TaskRunStatus::CompleteSuccess);
        assert_eq!(update.payload.attempts, 2);
        assert_eq!(
            update.payload.result_entity_type.as_deref(),
            Some("inference")
        );
        assert_eq!(
            update.payload.result_entity_id.as_deref(),
            Some("8a0d329d-72ca-4fd5-bd8d-2302f762f37d")
        );
        assert_eq!(update.payload.run, 4);
        assert_eq!(update.user_id, Some(8));
    }

    #[test]
    fn task_status_from_envelope_copies_identity_and_run() {
        #[derive(Clone, Debug, Serialize)]
        struct TestTask {
            id: i32,
        }

        impl Task for TestTask {
            fn task_type() -> &'static str {
                "test"
            }

            fn entity_type() -> &'static str {
                "capture"
            }

            fn entity_id(&self) -> i32 {
                self.id
            }
        }

        let envelope = TaskEnvelope::new(17, TestTask { id: 91 }, 4);
        let row = model::task_run_status::Model {
            id: 1,
            user_id: 17,
            envelope_id: envelope.envelope_id.clone(),
            run: 4,
            task_type: "test".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 91,
            task_payload: None,
            result_entity_type: None,
            result_entity_id: None,
            status_code: TaskRunStatus::InProgress.as_i32(),
            attempts: 2,
            created_at: timestamp(),
            processing_started_at: None,
            last_error_duration_ms: None,
            success_duration_ms: None,
            updated_at: timestamp(),
        };
        let event = TaskStatusEvent::from_envelope(&envelope, &row, None);

        assert_eq!(event.entity_type, "capture");
        assert_eq!(event.entity_id, 91);
        assert_eq!(event.payload.task_type, "test");
        assert_eq!(event.payload.status, TaskRunStatus::InProgress);
        assert_eq!(event.payload.attempts, 2);
        assert_eq!(event.payload.run, 4);
        assert_eq!(event.timestamp, timestamp());
        assert_eq!(event.user_id, Some(17));
    }

    #[test]
    fn availability_update_round_trips() {
        let update = AvailabilityEvent::new(
            ServerEventTypes::Availability,
            timestamp(),
            "capture",
            42,
            AvailabilityPayload {
                operation: AvailabilityState::Deleted,
            },
        );

        let encoded = serde_json::to_string(&update).unwrap();
        let decoded: AvailabilityEvent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, update);
    }
}
