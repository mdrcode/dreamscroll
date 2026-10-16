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
        let outcome = self
            .task_master
            .submit_illuminate(context, task, None)
            .await?;

        Self::log_outcome("illuminate", capture_id, outcome);
        Ok(())
    }

    /// A successful illumination is persisted before indexing.
    /// V1 remains raw-only and has no indexable document.
    pub async fn new_illumination(
        &self,
        illumination_task: &illuminate::IlluminationTask,
        backfill_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if matches!(
            illumination_task.prompt_version,
            crate::illumination::IlluminationVersion::V2
        ) {
            let capture_id = illumination_task.capture_id;
            let outcome = self
                .task_master
                .submit_search_index_from_worker(
                    search_index::SearchIndexTask {
                        capture_id,
                        entity_owner_id: illumination_task.entity_owner_id,
                    },
                    backfill_id,
                )
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
            task::SubmitOutcome::AlreadySubmittedBackfill { run_id, run_number } => {
                tracing::debug!(
                    task_type,
                    capture_id,
                    run_id = %run_id,
                    run_number,
                    "Beacon follow-up task was already submitted for this backfill"
                )
            }
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
    struct RecordingSearchIndexQueue(Arc<Mutex<Vec<task::TaskRun<search_index::SearchIndexTask>>>>);

    #[async_trait::async_trait]
    impl task::TaskQueue<search_index::SearchIndexTask> for RecordingSearchIndexQueue {
        async fn enqueue(
            &self,
            task_run: task::TaskRun<search_index::SearchIndexTask>,
        ) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("search-index queue mutex should not be poisoned")
                .push(task_run);
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
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V1,
        );
        let v2_task = illuminate::IlluminationTask::new(
            v2_capture_id,
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );

        beacon
            .new_illumination(&v1_task, None)
            .await
            .expect("v1 completion should not enqueue indexing");
        beacon
            .new_illumination(&v2_task, None)
            .await
            .expect("v2 completion should enqueue indexing");

        let queued = queued.lock().expect("queue mutex should not be poisoned");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task.entity_owner_id, 1);
        assert_eq!(queued[0].task.capture_id, v2_capture_id);
        assert_eq!(queued[0].backfill_id, None);
    }

    #[tokio::test]
    async fn backfill_descendants_keep_metadata_and_deduplicate_after_completion() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let queued = Arc::new(Mutex::new(Vec::new()));
        let task_master = Arc::new(
            task::TaskMaster::builder()
                .db(db.handle())
                .search_index_queue(RecordingSearchIndexQueue(Arc::clone(&queued)))
                .build()
                .unwrap(),
        );
        let beacon = Beacon::new(Arc::clone(&task_master));
        let illumination = illuminate::IlluminationTask::new(
            42,
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );
        beacon
            .new_illumination(&illumination, Some("test-backfill"))
            .await
            .unwrap();
        let run = queued.lock().expect("queue mutex should not be poisoned")[0].clone();
        assert_eq!(run.backfill_id.as_deref(), Some("test-backfill"));
        let attempt = task_master.begin_attempt(&run).await.unwrap().unwrap();
        task_master
            .finish_attempt(&run, attempt, &Ok(()))
            .await
            .unwrap();
        beacon
            .new_illumination(&illumination, Some("test-backfill"))
            .await
            .unwrap();
        assert_eq!(
            queued
                .lock()
                .expect("queue mutex should not be poisoned")
                .len(),
            1
        );

        // An ordinary completion is not tagged simply because a backfill has
        // previously indexed this same capture.
        beacon.new_illumination(&illumination, None).await.unwrap();
        let runs = queued.lock().expect("queue mutex should not be poisoned");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1].backfill_id, None);
        assert_eq!(runs[1].run_number, 2);
    }

    #[tokio::test]
    async fn a_live_ordinary_descendant_is_not_retagged_by_backfill_completion() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let queued = Arc::new(Mutex::new(Vec::new()));
        let task_master = Arc::new(
            task::TaskMaster::builder()
                .db(db.handle())
                .search_index_queue(RecordingSearchIndexQueue(Arc::clone(&queued)))
                .build()
                .unwrap(),
        );
        let beacon = Beacon::new(Arc::clone(&task_master));
        let illumination = illuminate::IlluminationTask::new(
            42,
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );
        beacon.new_illumination(&illumination, None).await.unwrap();
        beacon
            .new_illumination(&illumination, Some("test-backfill"))
            .await
            .unwrap();
        let run = queued.lock().expect("queue mutex should not be poisoned")[0].clone();
        assert_eq!(
            queued
                .lock()
                .expect("queue mutex should not be poisoned")
                .len(),
            1
        );
        assert_eq!(run.backfill_id, None);
        let context = auth::Context::from(auth::DreamscrollAuthUser::new_test_session(1));
        let stored = task_master
            .query_run_status(&context, &run.run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.backfill_id, None);
    }
}
