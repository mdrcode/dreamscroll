use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A Task is a serializable specification of a unit of work.
///
/// The entity is the conceptual target of the task. Its owner, type, and id
/// identify the work independently of who submitted it.
pub trait Task: Clone + Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str; // eg. "illuminate" or "spark"
    fn entity_type() -> &'static str; // eg. "capture" or "spark"
    fn entity_id(&self) -> i32;
    fn entity_owner_id(&self) -> i32;

    // Any further "payload" (e.g. which model to use, which prompt, etc)
    // is up to the concrete task implementation to define and serialize.
}

/// A TaskRun is one invocation of a Task and it may involve a small number of
/// retries. Its identity has two parts:
///
///  - logical_id: the combined identity of the target owner and task; it stays
///    the same across all runs of the same logical task.
///  - run_id: a globally unique identifier for this individual invocation.
///
/// If a previous run of the same logical task is already in progress, new
/// submissions are refused. Ordinary submissions can rerun settled work with
/// the same logical_id and a fresh run_id. A backfill's logical task is instead
/// submitted at most once, regardless of its run's eventual outcome.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskRun<T: Task> {
    /// Stable across all runs of the same task and target entity.
    pub logical_id: String,
    /// Globally unique identifier for this invocation; unchanged across retries.
    /// Task-owned raw/partial outputs use this stable key across retries.
    pub run_id: String,
    /// 1-based ordinal among runs of the same logical task.
    pub run_number: i32,
    /// Developer-managed backfill identity, inherited by descendant tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backfill_id: Option<String>,
    pub task: T,
}

impl<T: Task> TaskRun<T> {
    /// Create a new invocation with a fresh globally unique ID.
    pub fn new(task: T, run_number: i32) -> Self {
        Self {
            logical_id: Self::make_logical_id(&task),
            run_id: uuid::Uuid::new_v4().to_string(),
            run_number,
            backfill_id: None,
            task,
        }
    }

    /// Build the deterministic ID for logical work shared across runs.
    pub fn make_logical_id(task: &T) -> String {
        format!(
            "u{}-{}-{}{}",
            task.entity_owner_id(),
            T::task_type(),
            T::entity_type(),
            task.entity_id()
        )
    }
}

impl<T: Task> std::fmt::Debug for TaskRun<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("TaskRun");
        debug.field("task_type", &T::task_type());
        debug.field("logical_id", &self.logical_id);
        debug.field("run_id", &self.run_id);
        debug.field("run_number", &self.run_number);
        debug.field("backfill_id", &self.backfill_id);

        // Bounded preview of the serialized payload, so logs stay readable for
        // large tasks (e.g. a spark with many capture_ids).
        let json = serde_json::to_string(&self.task)
            .unwrap_or_else(|_| "<serialization error>".to_string());
        let payload_preview = if json.chars().count() > 200 {
            // Char-boundary-safe prefix, so a multi-byte UTF-8 boundary
            // can never panic.
            let preview: String = json.chars().take(200).collect();
            format!("{preview}...")
        } else {
            json
        };
        debug.field("payload", &payload_preview);

        debug.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestTask {
        id: i32,
        entity_owner_id: i32,
    }

    impl Task for TestTask {
        fn task_type() -> &'static str {
            "illuminate"
        }
        fn entity_type() -> &'static str {
            "capture"
        }
        fn entity_id(&self) -> i32 {
            self.id
        }
        fn entity_owner_id(&self) -> i32 {
            self.entity_owner_id
        }
    }

    /// The owner-scoped identity format is persisted and used by the in-flight guard.
    #[test]
    fn logical_id_format_is_stable() {
        assert_eq!(
            TaskRun::make_logical_id(&TestTask {
                id: 123,
                entity_owner_id: 1
            }),
            "u1-illuminate-capture123"
        );
    }

    /// The id names the work and target owner, so it stays stable across reruns.
    #[test]
    fn logical_id_excludes_the_run() {
        let task = TestTask {
            id: 5,
            entity_owner_id: 1,
        };
        let by_id = TaskRun::<TestTask>::make_logical_id(&task);
        let run1 = TaskRun::new(task.clone(), 1);
        let run7 = TaskRun::new(task, 7);

        assert_eq!(run1.logical_id, by_id);
        assert_eq!(run7.logical_id, by_id);
        assert_eq!(run1.run_number, 1);
        assert_eq!(run7.run_number, 7);
        assert_ne!(run1.run_id, run7.run_id);
    }

    #[test]
    fn logical_id_distinguishes_owners_and_entities() {
        let task = TestTask {
            id: 5,
            entity_owner_id: 1,
        };
        let logical_id = TaskRun::<TestTask>::make_logical_id(&task);
        let other_owner = TaskRun::<TestTask>::make_logical_id(&TestTask {
            id: 5,
            entity_owner_id: 2,
        });
        let other_entity = TaskRun::<TestTask>::make_logical_id(&TestTask {
            id: 6,
            entity_owner_id: 1,
        });

        assert_ne!(logical_id, other_owner);
        assert_ne!(logical_id, other_entity);
    }

    #[test]
    fn new_uses_task_owner_for_logical_identity() {
        let task_run = TaskRun::new(
            TestTask {
                id: 9,
                entity_owner_id: 3,
            },
            2,
        );

        assert_eq!(task_run.logical_id, "u3-illuminate-capture9");
        assert_eq!(task_run.run_number, 2);
        assert!(uuid::Uuid::parse_str(&task_run.run_id).is_ok());
    }

    #[test]
    fn backfill_metadata_survives_delivery_and_old_payloads_remain_untagged() {
        let mut run = TaskRun::new(
            TestTask {
                id: 5,
                entity_owner_id: 1,
            },
            1,
        );
        assert_eq!(run.backfill_id, None);
        let untagged = serde_json::to_value(&run).unwrap();
        assert!(untagged.get("backfill_id").is_none());
        assert_eq!(
            serde_json::from_value::<TaskRun<TestTask>>(untagged)
                .unwrap()
                .backfill_id,
            None
        );

        run.backfill_id = Some("test-backfill".to_string());
        let tagged = serde_json::to_value(&run).unwrap();
        let decoded: TaskRun<TestTask> = serde_json::from_value(tagged).unwrap();
        assert_eq!(decoded.backfill_id.as_deref(), Some("test-backfill"));
        assert_eq!(decoded.run_id, run.run_id);
        assert_eq!(decoded.logical_id, run.logical_id);
    }

    #[test]
    fn deserializing_without_run_identity_fields_fails() {
        let json = r#"{"task":{"id":5,"entity_owner_id":1}}"#;

        assert!(serde_json::from_str::<TaskRun<TestTask>>(json).is_err());
    }

    #[test]
    fn deserializing_without_a_task_fails() {
        let json = r#"{"logical_id":"u1-illuminate-capture5"}"#;

        assert!(serde_json::from_str::<TaskRun<TestTask>>(json).is_err());
    }

    #[test]
    fn serialization_round_trip_preserves_identity_and_payload() {
        let original = TaskRun::new(
            TestTask {
                id: 17,
                entity_owner_id: 4,
            },
            3,
        );

        let encoded = serde_json::to_string(&original).expect("TaskRun should serialize");
        let wire: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(wire["task"]["entity_owner_id"], 4);
        assert!(wire.get("entity_owner_id").is_none());
        assert_eq!(wire["logical_id"], "u4-illuminate-capture17");
        assert_eq!(wire["run_number"], 3);
        assert!(uuid::Uuid::parse_str(wire["run_id"].as_str().unwrap()).is_ok());
        assert!(wire.get("id").is_none());
        assert!(wire.get("run").is_none());
        assert!(wire.get("logical_task_id").is_none());
        assert!(wire.get("user_id").is_none());
        let decoded: TaskRun<TestTask> =
            serde_json::from_str(&encoded).expect("TaskRun should deserialize");

        assert_eq!(decoded.task.entity_owner_id, 4);
        assert_eq!(decoded.logical_id, original.logical_id);
        assert_eq!(decoded.run_number, 3);
        assert_eq!(decoded.task.id, 17);
    }

    #[test]
    fn debug_includes_owner_identity_and_payload() {
        let rendered = format!(
            "{:?}",
            TaskRun::new(
                TestTask {
                    id: 8,
                    entity_owner_id: 2
                },
                1
            )
        );

        assert!(rendered.contains("u2-illuminate-capture8"));
        assert!(rendered.contains("id\\\":8"));
    }

    /// Debug must not panic on a multi-byte payload cut at the 200-char preview.
    #[test]
    fn debug_truncates_a_multibyte_payload_without_panicking() {
        #[derive(Debug, Clone, Serialize)]
        struct Wide {
            text: String,
        }

        impl Task for Wide {
            fn task_type() -> &'static str {
                "wide"
            }
            fn entity_type() -> &'static str {
                "capture"
            }
            fn entity_id(&self) -> i32 {
                1
            }
            fn entity_owner_id(&self) -> i32 {
                1
            }
        }

        let task_run = TaskRun::new(
            Wide {
                text: "é".repeat(500),
            },
            1,
        );

        let rendered = format!("{task_run:?}");

        assert!(rendered.contains("..."), "a long payload is truncated");
    }
}
