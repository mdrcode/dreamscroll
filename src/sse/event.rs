use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::task::TaskRunStatus;

pub const CURRENT_SCHEMA_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerEventTypes {
    TaskStatus,
    Availability,
}

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
    pub operation: AvailabilityState,
}

pub type AvailabilityEvent = ServerEvent<AvailabilityPayload>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskStatusPayload {
    pub subchannel: String,
    pub status: TaskRunStatus,
    pub run: i32,
}

pub type TaskStatusEvent = ServerEvent<TaskStatusPayload>;

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
            },
        );

        assert_eq!(
            serde_json::to_string(&update).unwrap(),
            r#"{"schema_version":1,"event_type":"task_status","timestamp":"2026-09-21T18:42:10Z","entity_type":"capture","entity_id":42,"payload":{"subchannel":"illuminate","status":{"name":"complete_success","discriminant":4},"run":3}}"#
        );
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
