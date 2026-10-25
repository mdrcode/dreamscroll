use std::collections::HashMap;

use crate::{api, model, sse};

use super::{
    TaskRunStatus,
    taskruntracker::TaskRunTracker,
    timing::{self, TimingEstimator},
};

/// Replay of task status for a newly opened stream.
///
/// The read-side counterpart of `StatusPublisher`: it builds the events a live
/// stream would already have delivered, including the timing estimate a running
/// event carries. It never publishes and never writes.
///
/// Shares `TaskMaster`'s tracker (see `TaskMaster::status_catchup`), so there is one run
/// store per process; the owner-scoped latest-run query and the estimate live
/// there.
#[derive(Clone)]
pub struct StatusCatchup {
    tracker: TaskRunTracker,
    timing: TimingEstimator,
}

impl StatusCatchup {
    pub(super) fn new(tracker: TaskRunTracker, timing: TimingEstimator) -> Self {
        Self { tracker, timing }
    }

    /// Latest status per (entity, task type) for the given owner's entities.
    ///
    /// Scoping comes from `entity_owner_id`, so a caller cannot read rows beyond
    /// its own entities.
    pub async fn events(
        &self,
        entity_owner_id: i32,
        entity_type: &str,
        entity_ids: &[i32],
    ) -> Result<Vec<sse::TaskStatusUpdate>, api::ApiError> {
        let rows = self
            .tracker
            .query_latest_task_runs(entity_owner_id, entity_type, entity_ids)
            .await
            .map_err(api::ApiError::internal)?;

        // At most one aggregate lookup per distinct running task type.
        let mut estimates: HashMap<String, Option<timing::TaskTimingEstimate>> = HashMap::new();
        for row in rows.iter().filter(|row| is_running(row)) {
            if !estimates.contains_key(&row.task_type) {
                let estimate = self.timing.client_estimate(&row.task_type).await;
                estimates.insert(row.task_type.clone(), estimate);
            }
        }

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let estimate = if is_running(&row) {
                    estimates.get(&row.task_type).and_then(Option::as_ref)
                } else {
                    None
                };
                sse::TaskStatusUpdate::from_row(&row, estimate)
            })
            .collect())
    }
}

/// Only a status a worker can still act on has a live estimate worth replaying.
fn is_running(row: &model::task_run::Model) -> bool {
    row.status_code == TaskRunStatus::InProgress.as_i32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        database,
        illumination::IlluminationVersion,
        logic::{illuminate::IlluminationTask, search_index::SearchIndexTask},
        task::{TaskRun, TaskRunStatus, timing},
    };

    async fn insert_estimate(db: &database::DbHandle, task_type: &str, p50: i64) {
        model::task_run_timing::ActiveModel::builder()
            .set_task_type(task_type)
            .set_operation_type(timing::Measure::ProcessingSuccessful.as_ref())
            .set_sample_count(timing::MINIMUM_CLIENT_ESTIMATE_SAMPLES)
            .set_duration_ms_avg(p50)
            .set_duration_ms_p50(p50)
            .set_duration_ms_p75(p50)
            .set_duration_ms_p90(p50)
            .insert(&db.conn)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn catchup_replays_latest_status_with_an_estimate_only_while_running() {
        let Some(test_db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let db = test_db.handle();
        let catchup = StatusCatchup::new(
            TaskRunTracker::for_tests(db.clone(), None),
            TimingEstimator::new(db.clone()),
        );

        let illuminate = TaskRun::new(
            IlluminationTask::new(42, 1, "test-model", IlluminationVersion::V2),
            1,
        );
        catchup.tracker.insert_run(&illuminate).await.unwrap();
        catchup
            .tracker
            .set_status(&illuminate, TaskRunStatus::InProgress, 1, None)
            .await
            .unwrap();

        let search = TaskRun::new(
            SearchIndexTask {
                capture_id: 42,
                entity_owner_id: 1,
            },
            1,
        );
        catchup.tracker.insert_run(&search).await.unwrap();

        let other_owner = TaskRun::new(
            SearchIndexTask {
                capture_id: 42,
                entity_owner_id: 2,
            },
            1,
        );
        catchup.tracker.insert_run(&other_owner).await.unwrap();

        insert_estimate(&db, "illuminate", 12_000).await;

        let events = catchup.events(1, "capture", &[42]).await.unwrap();

        assert_eq!(
            events.len(),
            2,
            "one event per task type, scoped to the caller's entities"
        );
        assert!(events.iter().all(|event| event.entity_owner_id == Some(1)));
        let illuminate_event = events
            .iter()
            .find(|event| event.payload.task_type == "illuminate")
            .unwrap();
        assert_eq!(
            illuminate_event.payload.estimated_duration_ms_p50,
            Some(12_000),
            "a running run replays the estimate its live event would carry"
        );
        let search_event = events
            .iter()
            .find(|event| event.payload.task_type == "search_index")
            .unwrap();
        assert_eq!(
            search_event.payload.estimated_duration_ms_p50, None,
            "a queued run has no live estimate"
        );
    }
}
