use std::sync::Arc;

use crate::{auth, task};

use super::*;

/// Controller that coordinates follow-up task submission after capture events.
/// New captures enqueue illumination; successful v2 illuminations enqueue
/// vector indexing.
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
        task: illuminate::IlluminationTask,
    ) -> anyhow::Result<()> {
        let capture_id = task.capture_id;
        let outcome = self.task_master.submit_illuminate(context, task).await?;

        Self::log_outcome("illuminate", capture_id, outcome);
        Ok(())
    }

    /// A successful illumination is persisted before indexing.
    /// V1 remains raw-only and has no indexable document.
    pub async fn new_illumination(
        &self,
        user_id: i32,
        illumination_task: &illuminate::IlluminationTask,
    ) -> anyhow::Result<()> {
        if matches!(
            illumination_task.prompt_version,
            crate::illumination::IlluminationVersion::V2
        ) {
            let capture_id = illumination_task.capture_id;
            let outcome = self
                .task_master
                .submit_search_index_for_user(user_id, search_index::SearchIndexTask { capture_id })
                .await?;

            Self::log_outcome("search_index", capture_id, outcome);
        }

        Ok(())
    }

    fn log_outcome(task_type: &str, capture_id: i32, outcome: task::SubmitOutcome) {
        match outcome {
            task::SubmitOutcome::Enqueued { run_id, run_number } => tracing::debug!(
                task_type,
                capture_id,
                run_id = %run_id,
                run_number,
                "Beacon enqueued follow-up TaskRun"
            ),
            task::SubmitOutcome::RefusedAlreadyInFlight => tracing::debug!(
                task_type,
                capture_id,
                "Beacon follow-up task is already in flight"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct RecordingSearchIndexQueue(Arc<Mutex<Vec<(i32, i32)>>>);

    #[async_trait::async_trait]
    impl task::TaskQueue<search_index::SearchIndexTask> for RecordingSearchIndexQueue {
        async fn enqueue(
            &self,
            task_run: task::TaskRun<search_index::SearchIndexTask>,
        ) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("search-index queue mutex should not be poisoned")
                .push((task_run.user_id, task_run.task.capture_id));
            Ok(())
        }
    }

    #[tokio::test]
    async fn only_v2_illumination_enqueues_search_index() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let queued = Arc::new(Mutex::new(Vec::new()));
        let task_master = Arc::new(
            task::TaskMaster::builder()
                .db(db.handle())
                .search_index_queue(RecordingSearchIndexQueue(Arc::clone(&queued)))
                .build()
                .expect("TaskMaster should build with a database"),
        );
        let beacon = Beacon::new(task_master);
        let v1_capture_id = (uuid::Uuid::new_v4().as_u128() & i32::MAX as u128).max(1) as i32;
        let v2_capture_id = (uuid::Uuid::new_v4().as_u128() & i32::MAX as u128).max(1) as i32;
        let v1_task = illuminate::IlluminationTask::new(
            v1_capture_id,
            "test-model",
            crate::illumination::IlluminationVersion::V1,
        );
        let v2_task = illuminate::IlluminationTask::new(
            v2_capture_id,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );

        beacon
            .new_illumination(1, &v1_task)
            .await
            .expect("v1 completion should not enqueue indexing");
        beacon
            .new_illumination(1, &v2_task)
            .await
            .expect("v2 completion should enqueue indexing");

        assert_eq!(
            *queued.lock().expect("queue mutex should not be poisoned"),
            vec![(1, v2_capture_id)]
        );
    }
}
