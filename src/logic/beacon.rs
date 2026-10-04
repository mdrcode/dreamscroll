use std::sync::Arc;

use crate::{auth, task};

use super::*;

/// Controller that coordinates interaction between tasks via logical signals.
/// e.g. when a new capture is added to the system, the Beacon decides possible
/// follow up tasks such as illumination, search indexing, etc.
#[derive(Clone)]
pub struct Beacon {
    task_master: Arc<task::TaskMaster>,
}

impl Beacon {
    pub fn new(task_master: Arc<task::TaskMaster>) -> Self {
        Self { task_master }
    }

    /// A new capture has been added to the system.
    pub async fn new_capture(
        &self,
        context: &auth::Context,
        capture_id: i32,
    ) -> anyhow::Result<()> {
        let outcome = self
            .task_master
            .submit_illuminate(context, illuminate::IlluminationTask { capture_id })
            .await?;

        Self::log_outcome("illuminate", capture_id, outcome);
        Ok(())
    }

    /// Handle a successful illumination signal.
    /// This runs in a background worker, which has no authenticated request Context;
    /// pass the capture's persisted owner ID rather than the task envelope's user ID.
    pub async fn new_illumination(&self, user_id: i32, capture_id: i32) -> anyhow::Result<()> {
        let outcome = self
            .task_master
            .submit_search_index_for_user(user_id, search_index::SearchIndexTask { capture_id })
            .await?;

        Self::log_outcome("search_index", capture_id, outcome);
        Ok(())
    }

    fn log_outcome(task_type: &str, capture_id: i32, outcome: task::SubmitOutcome) {
        match outcome {
            task::SubmitOutcome::Enqueued { run } => {
                tracing::debug!(task_type, capture_id, run, "Beacon enqueued follow-up task")
            }
            task::SubmitOutcome::RefusedAlreadyInFlight => tracing::debug!(
                task_type,
                capture_id,
                "Beacon follow-up task is already in flight"
            ),
        }
    }
}
