use serde::Serialize;

use super::*;

/// A unified, high-level description of a background task.
///
/// This is the single "submit" contract the rest of the system uses to ask
/// for work. Each variant maps to one backend queue and one `task_type` in
/// the `task_status` table. `user_id` is carried here so a `Task` is fully
/// self-describing — it's needed for the `task_status` row even though the
/// backend queue tasks don't use it.
#[derive(Debug, Clone, Serialize)]
pub enum Task {
    Ingest { user_id: i32, capture_id: i32 },
    Illuminate { user_id: i32, capture_id: i32 },
    Spark { user_id: i32, capture_ids: Vec<i32> },
    SearchIndex { user_id: i32, capture_id: i32 },
}

impl Task {
    /// The `task_type` string stored in the `task_status` table.
    pub fn task_type(&self) -> &'static str {
        match self {
            Task::Ingest { .. } => "ingest",
            Task::Illuminate { .. } => "illumination",
            Task::Spark { .. } => "spark",
            Task::SearchIndex { .. } => "search_index",
        }
    }

    /// The `task_id` string stored in the `task_status` table.
    ///
    /// For single-capture tasks this is the capture_id; for `Spark` it is the
    /// capture_ids joined with `-` (matching the old `SparkTask::id`).
    pub fn task_id(&self) -> String {
        match self {
            Task::Ingest { capture_id, .. }
            | Task::Illuminate { capture_id, .. }
            | Task::SearchIndex { capture_id, .. } => capture_id.to_string(),
            Task::Spark { capture_ids, .. } => capture_ids
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("-"),
        }
    }

    /// The owning user, stored in the `task_status` row.
    pub fn user_id(&self) -> i32 {
        match self {
            Task::Ingest { user_id, .. }
            | Task::Illuminate { user_id, .. }
            | Task::Spark { user_id, .. }
            | Task::SearchIndex { user_id, .. } => *user_id,
        }
    }
}

impl TaskId for Task {
    fn id(&self) -> String {
        self.task_id()
    }
}
