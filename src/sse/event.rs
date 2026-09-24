use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::task::{Task, TaskEnvelope, TaskRunStatus};

pub const CURRENT_SCHEMA_VERSION: u8 = 1;

/// Base container for all server events streamed to clients via SSE
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEventTypes {
    Availability,
    TaskStatus,
}

// Concrete payload-enriched ServerEvent types
pub type AvailabilityEvent = ServerEvent<AvailabilityPayload>;
pub type TaskStatusEvent = ServerEvent<TaskStatusPayload>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerEvent<E> {
    pub schema_version: u8,
    pub event_type: ServerEventTypes,
    pub timestamp: DateTime<Utc>,
    pub entity_type: String,
    pub entity_id: i32,
    pub payload: E,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    Available,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AvailabilityPayload {
    pub operation: AvailabilityState, // TODO rename to state?
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskStatusPayload {
    pub subchannel: String,
    pub status: TaskRunStatus,
    pub attempts: i32,
    pub run: i32,
    #[serde(default)]
    pub user_id: i32, // TODO should this be in the parent container?
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
            payload,
        }
    }
}

impl ServerEvent<TaskStatusPayload> {
    pub fn from_envelope<T: Task>(
        envelope: &TaskEnvelope<T>,
        status: TaskRunStatus,
        attempts: i32,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self::task_status(
            timestamp,
            T::entity_type(),
            envelope.task.entity_id(),
            T::task_type(),
            status,
            attempts,
            envelope.run,
            envelope.user_id,
        )
    }

    pub fn task_status(
        timestamp: DateTime<Utc>,
        entity_type: impl Into<String>,
        entity_id: i32,
        subchannel: impl Into<String>,
        status: TaskRunStatus,
        attempts: i32,
        run: i32,
        user_id: i32,
    ) -> Self {
        ServerEvent::new(
            ServerEventTypes::TaskStatus,
            timestamp,
            entity_type,
            entity_id,
            TaskStatusPayload {
                subchannel: subchannel.into(),
                status,
                attempts,
                run,
                user_id,
            },
        )
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
                subchannel: "illuminate".to_string(),
                run: 3,
                status: TaskRunStatus::CompleteSuccess,
                attempts: 1,
                user_id: 7,
            },
        );

        assert_eq!(
            serde_json::to_string(&update).unwrap(),
            r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":{"name":"complete_success","discriminant":4},"attempts":1,"run":3,"user_id":7}}"#
        );
    }

    #[test]
    fn task_status_constructor_sets_schema_and_routing_fields() {
        let update = TaskStatusEvent::task_status(
            timestamp(),
            "capture",
            91,
            "illuminate",
            TaskRunStatus::ErrorWillRetry,
            2,
            4,
            8,
        );

        assert_eq!(update.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(update.event_type, ServerEventTypes::TaskStatus);
        assert_eq!(update.entity_type, "capture");
        assert_eq!(update.entity_id, 91);
        assert_eq!(update.payload.subchannel, "illuminate");
        assert_eq!(update.payload.status, TaskRunStatus::ErrorWillRetry);
        assert_eq!(update.payload.attempts, 2);
        assert_eq!(update.payload.run, 4);
        assert_eq!(update.payload.user_id, 8);
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
        let event =
            TaskStatusEvent::from_envelope(&envelope, TaskRunStatus::InProgress, 2, timestamp());

        assert_eq!(event.entity_type, "capture");
        assert_eq!(event.entity_id, 91);
        assert_eq!(event.payload.subchannel, "test");
        assert_eq!(event.payload.status, TaskRunStatus::InProgress);
        assert_eq!(event.payload.attempts, 2);
        assert_eq!(event.payload.run, 4);
        assert_eq!(event.timestamp, timestamp());
        assert_eq!(event.payload.user_id, 17);
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
