use std::sync::Arc;

use crate::{auth, task};

use super::*;

/// Controller that coordinates follow-up task submission after capture events.
/// New captures enqueue illumination; successful projected v2 illuminations
/// enqueue vector indexing.
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

    /// A successful projected illumination is persisted before anything downstream sees it.
    ///
    /// `illumination_id` is the projection that was written, or `None` for raw-only tasks (including
    /// V1) — those have no observations to link and no new indexable document.
    pub async fn new_illumination(
        &self,
        illumination_task: &illuminate::IlluminationTask,
        illumination_id: Option<i32>,
        backfill_id: Option<&str>,
    ) -> anyhow::Result<()> {
        if illumination_task.persist_projection
            && matches!(
                illumination_task.prompt_version,
                crate::illumination::IlluminationVersion::V2
            )
        {
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

        // One illumination is one unit of graph work, and the queue's retry boundary is then the same
        // as every other operation's: a failed link costs this reading, not the capture's history.
        if let Some(illumination_id) = illumination_id {
            let outcome = self
                .task_master
                .submit_knowledge_link_from_worker(
                    knowledge_link::KnowledgeLinkTask {
                        illumination_id,
                        entity_owner_id: illumination_task.entity_owner_id,
                    },
                    backfill_id,
                )
                .await?;

            Self::log_outcome("knowledge_link", illumination_id, outcome);
        }

        Ok(())
    }

    /// An illumination has just been "linked" into the knowledge graph, which
    /// means that its "observed" entities have been properly incorporated into
    /// the formal set of knowledge entities.
    ///
    /// Now, as a result, it's time to resolve the identities of any new or
    /// updated entities.
    pub async fn knowledge_linked(
        &self,
        illumination_id: i32,
        entity_owner_id: i32,
        backfill_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let outcome = self
            .task_master
            .submit_identity_resolve_from_worker(
                identity_resolve::IdentityResolveTask {
                    illumination_id,
                    entity_owner_id,
                },
                backfill_id,
            )
            .await?;

        Self::log_outcome("identity_resolve", illumination_id, outcome);
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

    use crate::test_support::db_or_skip;

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct RecordingSearchIndexQueue(Arc<Mutex<Vec<task::TaskRun<search_index::SearchIndexTask>>>>);

    #[derive(Debug, Clone, Default)]
    struct RecordingKnowledgeLinkQueue(
        Arc<Mutex<Vec<task::TaskRun<knowledge_link::KnowledgeLinkTask>>>>,
    );

    #[async_trait::async_trait]
    impl task::TaskQueue<knowledge_link::KnowledgeLinkTask> for RecordingKnowledgeLinkQueue {
        async fn enqueue(
            &self,
            task_run: task::TaskRun<knowledge_link::KnowledgeLinkTask>,
        ) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("knowledge-link queue mutex should not be poisoned")
                .push(task_run);
            Ok(())
        }
    }

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

    /// What the recorded search-index queue received, shared with the assertions.
    type Queued = Arc<Mutex<Vec<task::TaskRun<search_index::SearchIndexTask>>>>;

    fn search_queue_beacon(
        db: crate::database::DbHandle,
    ) -> (Arc<task::TaskMaster>, Beacon, Queued) {
        let queued = Arc::new(Mutex::new(Vec::new()));
        let task_master = Arc::new(
            task::TaskMaster::builder()
                .db(db)
                .search_index_queue(RecordingSearchIndexQueue(Arc::clone(&queued)))
                .build()
                .expect("TaskMaster should build with a database"),
        );
        (Arc::clone(&task_master), Beacon::new(task_master), queued)
    }

    #[tokio::test]
    async fn only_projected_v2_illumination_enqueues_search_index() {
        db_or_skip!(db);
        let (_task_master, beacon, queued) = search_queue_beacon(db);
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
        assert!(v2_task.persist_projection);
        let mut raw_only_task = v2_task.clone();
        raw_only_task.persist_projection = false;
        beacon
            .new_illumination(&raw_only_task, None, None)
            .await
            .expect("raw-only v2 completion should not enqueue indexing");
        beacon
            .new_illumination(&raw_only_task, None, Some("raw-only-backfill"))
            .await
            .expect("raw-only backfill completion should not enqueue indexing");
        assert!(
            queued
                .lock()
                .expect("queue mutex should not be poisoned")
                .is_empty()
        );

        beacon
            .new_illumination(&v1_task, None, None)
            .await
            .expect("v1 completion should not enqueue indexing");
        beacon
            .new_illumination(&v2_task, Some(7), None)
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
        db_or_skip!(db);
        let (task_master, beacon, queued) = search_queue_beacon(db);
        let illumination = illuminate::IlluminationTask::new(
            42,
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );
        beacon
            .new_illumination(&illumination, Some(11), Some("test-backfill"))
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
            .new_illumination(&illumination, Some(11), Some("test-backfill"))
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
        beacon
            .new_illumination(&illumination, Some(11), None)
            .await
            .unwrap();
        let runs = queued.lock().expect("queue mutex should not be poisoned");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1].backfill_id, None);
        assert_eq!(runs[1].run_number, 2);
    }

    #[tokio::test]
    async fn a_live_ordinary_descendant_is_not_retagged_by_backfill_completion() {
        db_or_skip!(db);
        let (task_master, beacon, queued) = search_queue_beacon(db);
        let illumination = illuminate::IlluminationTask::new(
            42,
            1,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );
        beacon
            .new_illumination(&illumination, Some(11), None)
            .await
            .unwrap();
        beacon
            .new_illumination(&illumination, Some(11), Some("test-backfill"))
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

    /// The hook the queue hangs off: a persisted illumination names the work, and a raw-only one
    /// has nothing to name.
    #[tokio::test]
    async fn a_persisted_illumination_enqueues_its_knowledge_link() {
        db_or_skip!(db);
        let queued = Arc::new(Mutex::new(Vec::new()));
        let task_master = Arc::new(
            task::TaskMaster::builder()
                .db(db)
                .knowledge_link_queue(RecordingKnowledgeLinkQueue(Arc::clone(&queued)))
                .build()
                .unwrap(),
        );
        let beacon = Beacon::new(task_master);
        let task = illuminate::IlluminationTask::new(
            42,
            9,
            "test-model",
            crate::illumination::IlluminationVersion::V2,
        );

        beacon
            .new_illumination(&task, None, None)
            .await
            .expect("a raw-only result must not enqueue a link");
        assert!(
            queued
                .lock()
                .expect("queue mutex should not be poisoned")
                .is_empty()
        );

        beacon
            .new_illumination(&task, Some(77), Some("bf-1"))
            .await
            .expect("a persisted illumination should enqueue its link");

        let queued = queued.lock().expect("queue mutex should not be poisoned");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task.illumination_id, 77);
        assert_eq!(queued[0].task.entity_owner_id, 9);
        assert_eq!(queued[0].backfill_id.as_deref(), Some("bf-1"));
    }
}
