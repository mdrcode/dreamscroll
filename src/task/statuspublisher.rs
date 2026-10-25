use std::sync::Arc;

use crate::{model, sse};

use super::timing::TimingEstimator;

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
    /// logged because the database write already succeeded.
    ///
    /// The row is authoritative for every field the event carries.
    pub async fn publish(&self, row: &model::task_run::Model) {
        let Some(notifier) = &self.notifier else {
            return;
        };

        let estimate = self.timing.client_estimate(&row.task_type).await;
        let Some(event) = sse::TaskStatusUpdate::from_row(row, estimate.as_ref()) else {
            tracing::warn!(
                run_id = %row.run_id,
                status_code = row.status_code,
                "Skipping task-status notification for an unknown status"
            );
            return;
        };
        if let Err(error) = notifier.notify_task_status(&event).await {
            tracing::debug!(
                run_id = %row.run_id,
                error = ?error,
                "Failed to publish task-status notification"
            );
        }
    }
}
