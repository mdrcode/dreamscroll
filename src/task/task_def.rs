use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A Task is a serializable specification of a unit of work.
///
/// An entity is the conceptual target of the task, upon which it operates.
/// We don't care what the entity actually is, we track it only by its type
/// and id. The entity's identity contributes to the unique identity of the
/// task (represented via its `TaskRun`). Additionally, the entity is a
/// very convenient handle for querying out standing tasks, e.g. "What are all
/// the ongoing/completed tasks for capture 42?".
pub trait Task: Clone + Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str; // eg. "illuminate" or "spark"
    fn entity_type() -> &'static str; // eg. "capture" or "spark"
    fn entity_id(&self) -> i32;

    // Any further "payload" (e.g. which model to use, which prompt, etc)
    // is up to the concrete task implementation to define and serialize.
}

/// A TaskRun is one invocation of a Task on behalf of a user, and it may
/// involve a small number of retries. Its identity has two parts:
///
///  - logical_id: the combined identity of the user and task; it stays the same
///    across all runs of the same logical task.
///  - run_id: a globally unique identifier for this individual invocation.
///
/// If a previous run of the same logical task is already in progress, new
/// submissions are refused. But if all prior runs are complete, submitting the
/// same task again creates a new run. In that case, the logical_id remains the
/// same while the run_id changes.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskRun<T: Task> {
    pub user_id: i32,
    /// Stable across all runs of the same user's task and entity.
    pub logical_id: String,
    /// Globally unique identifier for this invocation; unchanged across retries.
    pub run_id: String,
    /// 1-based ordinal among runs of the same logical task.
    pub run_number: i32,
    pub task: T,
}

impl<T: Task> TaskRun<T> {
    /// Create a new invocation with a fresh globally unique ID.
    pub fn new(user_id: i32, task: T, run_number: i32) -> Self {
        Self {
            user_id,
            logical_id: Self::make_logical_id(user_id, &task),
            run_id: uuid::Uuid::new_v4().to_string(),
            run_number,
            task,
        }
    }

    /// Build the deterministic ID for logical work shared across runs.
    pub fn make_logical_id(user_id: i32, task: &T) -> String {
        format!(
            "u{}-{}-{}{}",
            user_id,
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
        debug.field("user_id", &self.user_id);
        debug.field("logical_id", &self.logical_id);
        debug.field("run_id", &self.run_id);
        debug.field("run_number", &self.run_number);

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
    }

    /// The id format is persisted, so changing it silently would orphan every
    /// existing `task_run_status` row.
    #[test]
    fn logical_id_format_is_stable() {
        assert_eq!(
            TaskRun::make_logical_id(1, &TestTask { id: 123 }),
            "u1-illuminate-capture123"
        );
    }

    /// The id names the *work*, so the same task yields the same id regardless
    /// of run. That is what lets a rerun target the same logical task.
    #[test]
    fn logical_id_excludes_the_run() {
        let by_id = TaskRun::<TestTask>::make_logical_id(1, &TestTask { id: 5 });
        let run1 = TaskRun::new(1, TestTask { id: 5 }, 1);
        let run7 = TaskRun::new(1, TestTask { id: 5 }, 7);

        assert_eq!(run1.logical_id, by_id);
        assert_eq!(run7.logical_id, by_id);
        assert_eq!(run1.run_number, 1);
        assert_eq!(run7.run_number, 7);
        assert_ne!(run1.run_id, run7.run_id);
    }

    #[test]
    fn logical_id_distinguishes_users_and_entities() {
        let a = TaskRun::<TestTask>::make_logical_id(1, &TestTask { id: 5 });
        let other_user = TaskRun::<TestTask>::make_logical_id(2, &TestTask { id: 5 });
        let other_entity = TaskRun::<TestTask>::make_logical_id(1, &TestTask { id: 6 });

        assert_ne!(a, other_user);
        assert_ne!(a, other_entity);
    }

    #[test]
    fn new_assigns_run_id_and_number() {
        let task_run = TaskRun::new(3, TestTask { id: 9 }, 2);

        assert_eq!(task_run.user_id, 3);
        assert_eq!(task_run.run_number, 2);
        assert!(uuid::Uuid::parse_str(&task_run.run_id).is_ok());
    }

    #[test]
    fn deserializing_without_run_identity_fields_fails() {
        let json = r#"{"user_id":1,"task":{"id":5}}"#;

        assert!(serde_json::from_str::<TaskRun<TestTask>>(json).is_err());
    }

    #[test]
    fn deserializing_without_a_task_fails() {
        let json = r#"{"user_id":1,"logical_id":"u1-illuminate-capture5"}"#;

        assert!(serde_json::from_str::<TaskRun<TestTask>>(json).is_err());
    }

    #[test]
    fn serialization_round_trip_preserves_identity_and_payload() {
        let original = TaskRun::new(4, TestTask { id: 17 }, 3);

        let encoded = serde_json::to_string(&original).expect("TaskRun should serialize");
        let wire: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(wire["logical_id"], "u4-illuminate-capture17");
        assert_eq!(wire["run_number"], 3);
        assert!(uuid::Uuid::parse_str(wire["run_id"].as_str().unwrap()).is_ok());
        assert!(wire.get("id").is_none());
        assert!(wire.get("run").is_none());
        assert!(wire.get("logical_task_id").is_none());
        let decoded: TaskRun<TestTask> =
            serde_json::from_str(&encoded).expect("TaskRun should deserialize");

        assert_eq!(decoded.user_id, 4);
        assert_eq!(decoded.logical_id, original.logical_id);
        assert_eq!(decoded.run_number, 3);
        assert_eq!(decoded.task.id, 17);
    }

    #[test]
    fn debug_includes_identity_and_payload_preview() {
        let rendered = format!("{:?}", TaskRun::new(2, TestTask { id: 8 }, 1));

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
        }

        let task_run = TaskRun::new(
            1,
            Wide {
                text: "é".repeat(500),
            },
            1,
        );

        let rendered = format!("{task_run:?}");

        assert!(rendered.contains("..."), "a long payload is truncated");
    }
}
