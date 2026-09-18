use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// A Task is a serializable specification of a unit of work.
///
/// An entity is the conceptual target of the task, upon which it operates.
/// We don't care what the entity actually is, we track it only by its type
/// and id. The entity's identity contributes to the unique identity of the
/// task (represented via the `TaskEnvelope`). Additionally, the entity is a
/// very convenient handle for querying out standing tasks, e.g. "What are all
/// the ongoing/completed tasks for capture 42?".
pub trait Task: Clone + Debug + Send + Sync + Serialize {
    fn task_type() -> &'static str; // eg. "illuminate" or "spark"
    fn entity_type() -> &'static str; // eg. "capture" or "spark"
    fn entity_id(&self) -> i32;

    // Any further "payload" (e.g. which model to use, which prompt, etc)
    // is up to the concrete task implementation to define and serialize.
}

/// A TaskEnvelope is a properly wrapped Task which has been submitted to a TaskQueue.
///
/// An Envelope refers to a specific run of a logical task. If one run is already
/// in flight, then the system rejects duplicate submission of the same logical Task.
/// However, once the run completes (either CompleteSuccess or CompleteFailure),
/// then it can be resubmitted, which achieve a "re-run" of the task.
///
/// `envelope_id` identifies the *logical* task; `run` identifies one attempt to
/// carry it out. Together they key a `task_run_status` row, so a rerun of
/// Complete work (regardless of CompleteSuccess or CompleteFailure) is a new
/// run rather than an overwrite.
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskEnvelope<T: Task> {
    pub user_id: i32,
    pub envelope_id: String,
    /// Which run of this logical task this envelope carries, counting from 1.
    #[serde(default = "first_run")]
    pub run: i32,
    pub task: T,
}

fn first_run() -> i32 {
    1
}

impl<T: Task> TaskEnvelope<T> {
    /// Build an envelope for a run of a task.
    ///
    /// `run` counts from 1; callers get it from the latest `task_run_status` row.
    pub fn new(user_id: i32, task: T, run: i32) -> Self {
        Self {
            user_id,
            envelope_id: Self::make_envelope_id(user_id, &task),
            run,
            task,
        }
    }

    /// Build the deterministic identity for a logical task, e.g.
    /// `u1-illuminate-capture123`. Encodes user_id + task_type + entity.
    ///
    /// Deliberately *excludes* the run: the id names the work, not one attempt
    /// at it. It is a static so callers can compute the id before an envelope
    /// exists, which is what lets a submitter look up the latest run first.
    pub fn make_envelope_id(user_id: i32, task: &T) -> String {
        format!(
            "u{}-{}-{}{}",
            user_id,
            T::task_type(),
            T::entity_type(),
            task.entity_id()
        )
    }
}

impl<T: Task> std::fmt::Debug for TaskEnvelope<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("TaskEnvelope");
        debug.field("task_type", &T::task_type());
        debug.field("user_id", &self.user_id);
        debug.field("envelope_id", &self.envelope_id);
        debug.field("run", &self.run);

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
    fn envelope_id_format_is_stable() {
        assert_eq!(
            TaskEnvelope::make_envelope_id(1, &TestTask { id: 123 }),
            "u1-illuminate-capture123"
        );
    }

    /// The id names the *work*, so the same task yields the same id regardless
    /// of run. That is what lets a rerun target the same logical task.
    #[test]
    fn envelope_id_excludes_the_run() {
        let by_id = TaskEnvelope::<TestTask>::make_envelope_id(1, &TestTask { id: 5 });
        let run1 = TaskEnvelope::new(1, TestTask { id: 5 }, 1);
        let run7 = TaskEnvelope::new(1, TestTask { id: 5 }, 7);

        assert_eq!(run1.envelope_id, by_id);
        assert_eq!(run7.envelope_id, by_id);
        assert_ne!(run1.run, run7.run);
    }

    #[test]
    fn envelope_id_distinguishes_users_and_entities() {
        let a = TaskEnvelope::<TestTask>::make_envelope_id(1, &TestTask { id: 5 });
        let other_user = TaskEnvelope::<TestTask>::make_envelope_id(2, &TestTask { id: 5 });
        let other_entity = TaskEnvelope::<TestTask>::make_envelope_id(1, &TestTask { id: 6 });

        assert_ne!(a, other_user);
        assert_ne!(a, other_entity);
    }

    #[test]
    fn new_wraps_the_task_and_defaults_to_the_given_run() {
        let envelope = TaskEnvelope::new(3, TestTask { id: 9 }, 2);

        assert_eq!(envelope.user_id, 3);
        assert_eq!(envelope.run, 2);
        assert_eq!(envelope.task.id, 9);
    }

    /// A deserialized envelope without a `run` must not land on run 0, which
    /// would collide with nothing and silently create an off-by-one history.
    #[test]
    fn deserializing_without_a_run_defaults_to_one() {
        let json = r#"{"user_id":1,"envelope_id":"u1-illuminate-capture5","task":{"id":5}}"#;

        let envelope: TaskEnvelope<TestTask> =
            serde_json::from_str(json).expect("envelope should deserialize");

        assert_eq!(envelope.run, 1);
    }

    #[test]
    fn deserializing_without_a_task_fails() {
        let json = r#"{"user_id":1,"envelope_id":"u1-illuminate-capture5"}"#;

        assert!(serde_json::from_str::<TaskEnvelope<TestTask>>(json).is_err());
    }

    #[test]
    fn serialization_round_trip_preserves_identity_and_payload() {
        let original = TaskEnvelope::new(4, TestTask { id: 17 }, 3);

        let encoded = serde_json::to_string(&original).expect("envelope should serialize");
        let decoded: TaskEnvelope<TestTask> =
            serde_json::from_str(&encoded).expect("envelope should deserialize");

        assert_eq!(decoded.user_id, 4);
        assert_eq!(decoded.envelope_id, original.envelope_id);
        assert_eq!(decoded.run, 3);
        assert_eq!(decoded.task.id, 17);
    }

    #[test]
    fn debug_includes_identity_and_payload_preview() {
        let rendered = format!("{:?}", TaskEnvelope::new(2, TestTask { id: 8 }, 1));

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

        let envelope = TaskEnvelope::new(
            1,
            Wide {
                text: "é".repeat(500),
            },
            1,
        );

        let rendered = format!("{envelope:?}");

        assert!(rendered.contains("..."), "a long payload is truncated");
    }
}
