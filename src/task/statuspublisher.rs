use std::sync::Arc;

use crate::{database, model, sse};

use super::{Task, TaskRun, timing};

/// Publishes best-effort status hints after a `task_runs` write.
///
/// Split out of `TaskRunTracker` so the tracker stays a persistence component:
/// the timing-estimate lookup, SSE event construction, and the "a failed
/// notification never fails the write" policy all live here.
#[derive(Clone)]
pub struct StatusPublisher {
    db: database::DbHandle,
    notifier: Option<Arc<dyn sse::ServerEventNotifier>>,
}

impl StatusPublisher {
    pub fn new(
        db: database::DbHandle,
        notifier: Option<Arc<dyn sse::ServerEventNotifier>>,
    ) -> Self {
        Self { db, notifier }
    }

    /// Notify subscribers about one persisted status row. Best effort: errors are
    /// logged at debug level because the database write already succeeded.
    pub async fn publish<T: Task>(&self, task_run: &TaskRun<T>, row: &model::task_run::Model) {
        let Some(notifier) = &self.notifier else {
            return;
        };

        let estimate = timing::get_timing_estimate(
            &self.db,
            T::task_type(),
            timing::Measure::ProcessingSuccessful,
        )
        .await;
        let event = sse::TaskStatusEvent::from_task_run(task_run, row, estimate.as_ref());
        if let Err(error) = notifier.notify_task_status(&event).await {
            tracing::debug!(
                task_run = ?task_run,
                error = ?error,
                "Failed to publish task-status notification"
            );
        }
    }
}
