use std::sync::Arc;

use crate::{model, sse};

use super::{Task, TaskRun, timing::TimingEstimator};

/// Delivers best-effort status hints after a `task_runs` write.
///
/// Split out of `TaskRunTracker` so the tracker stays persistence-only, while this
/// owns the estimate, the SSE event, and the "a failed notification never fails
/// the write" policy.
#[derive(Clone)]
pub struct StatusPublisher {
    timing: TimingEstimator,
    notifier: Option<Arc<dyn sse::ServerEventNotifier>>,
}

impl StatusPublisher {
    pub fn new(
        timing: TimingEstimator,
        notifier: Option<Arc<dyn sse::ServerEventNotifier>>,
    ) -> Self {
        Self { timing, notifier }
    }

    /// Notify subscribers about one persisted status row. Best effort: errors are
    /// logged at debug level because the database write already succeeded.
    pub async fn publish<T: Task>(&self, task_run: &TaskRun<T>, row: &model::task_run::Model) {
        let Some(notifier) = &self.notifier else {
            return;
        };

        let estimate = self.timing.client_estimate(T::task_type()).await;
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
