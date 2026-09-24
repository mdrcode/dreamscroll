use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{database, model, sse};

use super::*;

/// Manages all direct reads/writes to `task_run_status` in the db.
///
/// Publishes a best-effort status hint after each successful row
/// insert/update.
///
/// TaskMaster retains Task lifecycle policy and queue coordination.
#[derive(Clone)]
pub struct TaskRunTracker {
    db: database::DbHandle,
    notifier: Option<std::sync::Arc<dyn sse::ServerEventNotifier>>,
}

/// True when a `DbErr` is a unique-constraint violation. The
/// `(envelope_id, run)` unique index is the real guard against a duplicate
/// submission, so this is expected, not exceptional.
fn is_unique_violation(err: &sea_orm::DbErr) -> bool {
    matches!(
        err.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    )
}

impl TaskRunTracker {
    pub fn new(
        db: database::DbHandle,
        notifier: Option<std::sync::Arc<dyn sse::ServerEventNotifier>>,
    ) -> Self {
        Self { db, notifier }
    }

    /// Insert a task run in its initial `Queued` state with zero attempts.
    ///
    /// Returns `Ok(false)` if `(envelope_id, run)`already exists — this covers
    /// the *lost-a-race* scenario, where a concurrent submit claimed the run
    /// first. The unique index, not the read, is what makes correctness here.
    pub async fn create_run<T: Task>(&self, envelope: &TaskEnvelope<T>) -> anyhow::Result<bool> {
        let db = &self.db;

        let task = &envelope.task;

        let result = model::task_run_status::ActiveModel::builder()
            .set_task_type(T::task_type())
            .set_envelope_id(envelope.envelope_id.as_str())
            .set_run(envelope.run)
            .set_entity_type(T::entity_type())
            .set_entity_id(task.entity_id())
            .set_user_id(envelope.user_id)
            .set_status_code(TaskRunStatus::Queued.as_i32())
            .set_attempts(0)
            .save(&db.conn)
            .await;

        match result {
            Ok(_) => {
                self.notify_status(envelope, TaskRunStatus::Queued, 0).await;
                Ok(true)
            }
            Err(err) if is_unique_violation(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    /// Update the row for an existing `(envelope_id, run)`. No-op if missing.
    pub async fn update_run<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: TaskRunStatus,
        attempts: i32,
    ) -> anyhow::Result<()> {
        let db = &self.db;

        let result = model::task_run_status::Entity::update_many()
            .col_expr(
                model::task_run_status::Column::StatusCode,
                sea_orm::sea_query::Expr::value(status.as_i32()),
            )
            .col_expr(
                model::task_run_status::Column::Attempts,
                sea_orm::sea_query::Expr::value(attempts),
            )
            .col_expr(
                model::task_run_status::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(chrono::Utc::now()),
            )
            .filter(model::task_run_status::Column::EnvelopeId.eq(envelope.envelope_id.as_str()))
            .filter(model::task_run_status::Column::Run.eq(envelope.run))
            .exec(&db.conn)
            .await?;

        if result.rows_affected > 0 {
            self.notify_status(envelope, status, attempts).await;
        }

        Ok(())
    }

    async fn notify_status<T: Task>(
        &self,
        envelope: &TaskEnvelope<T>,
        status: TaskRunStatus,
        attempts: i32,
    ) {
        let Some(notifier) = &self.notifier else {
            return;
        };

        let event = sse::TaskStatusEvent::from_envelope(envelope, status, attempts);
        if let Err(error) = notifier.notify_task_status(&event).await {
            tracing::debug!(
                envelope = ?envelope,
                error = ?error,
                "Failed to publish task-status notification"
            );
        }
    }

    /// The most recent run of a logical task, or `None` if it has never run.
    pub async fn query_latest_run(
        &self,
        envelope_id: &str,
    ) -> anyhow::Result<Option<model::task_run_status::Model>> {
        let db = &self.db;

        let row = model::task_run_status::Entity::find()
            .filter(model::task_run_status::Column::EnvelopeId.eq(envelope_id))
            .order_by_desc(model::task_run_status::Column::Run)
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// The status row for one specific run, used by `begin_attempt`.
    pub async fn query_run_status(
        &self,
        envelope_id: &str,
        run: i32,
    ) -> anyhow::Result<Option<model::task_run_status::Model>> {
        let db = &self.db;

        let row = model::task_run_status::Entity::find()
            .filter(model::task_run_status::Column::EnvelopeId.eq(envelope_id))
            .filter(model::task_run_status::Column::Run.eq(run))
            .one(&db.conn)
            .await?;

        Ok(row)
    }

    /// Latest task statuses for several entities of the same type and user.
    pub async fn query_latest_status_for_entities(
        &self,
        user_id: i32,
        entity_type: &str,
        entity_ids: &[i32],
    ) -> anyhow::Result<Vec<model::task_run_status::Model>> {
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = model::task_run_status::Entity::find()
            .filter(model::task_run_status::Column::UserId.eq(user_id))
            .filter(model::task_run_status::Column::EntityType.eq(entity_type))
            .filter(model::task_run_status::Column::EntityId.is_in(entity_ids.iter().copied()))
            .all(&self.db.conn)
            .await?;

        Ok(latest_runs_per_task(rows))
    }
}

/// Reduce rows to the latest run per logical task. No status is filtered out:
/// completed work is reported alongside in-flight and failed work.
fn latest_runs_per_task(
    rows: Vec<model::task_run_status::Model>,
) -> Vec<model::task_run_status::Model> {
    let mut latest: std::collections::HashMap<String, model::task_run_status::Model> =
        std::collections::HashMap::new();

    for row in rows {
        match latest.get(&row.envelope_id) {
            Some(existing) if existing.run >= row.run => {}
            _ => {
                latest.insert(row.envelope_id.clone(), row);
            }
        }
    }

    latest.into_values().collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct RecordingNotifier(Arc<Mutex<Vec<sse::TaskStatusEvent>>>);

    #[async_trait::async_trait]
    impl sse::ServerEventNotifier for RecordingNotifier {
        async fn notify_task_status(&self, event: &sse::TaskStatusEvent) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct FailingNotifier;

    #[async_trait::async_trait]
    impl sse::ServerEventNotifier for FailingNotifier {
        async fn notify_task_status(&self, _event: &sse::TaskStatusEvent) -> anyhow::Result<()> {
            anyhow::bail!("simulated notification failure")
        }
    }

    /// A minimal task, so tracker tests don't depend on a real task type.
    #[derive(Debug, Clone, serde::Serialize)]
    struct TestTask {
        id: i32,
    }

    impl Task for TestTask {
        fn task_type() -> &'static str {
            "test"
        }
        fn entity_type() -> &'static str {
            "capture"
        }
        fn entity_id(&self) -> i32 {
            self.id
        }
    }

    fn envelope(user_id: i32, id: i32, run: i32) -> TaskEnvelope<TestTask> {
        TaskEnvelope::new(user_id, TestTask { id }, run)
    }

    async fn create_run_with_status<T: Task>(
        tracker: &TaskRunTracker,
        envelope: &TaskEnvelope<T>,
        status: TaskRunStatus,
        attempts: i32,
    ) -> anyhow::Result<bool> {
        let created = tracker.create_run(envelope).await?;
        if created && (status != TaskRunStatus::Queued || attempts != 0) {
            tracker.update_run(envelope, status, attempts).await?;
        }
        Ok(created)
    }

    fn row(envelope_id: &str, run: i32, status: TaskRunStatus) -> model::task_run_status::Model {
        model::task_run_status::Model {
            id: 0,
            user_id: 1,
            envelope_id: envelope_id.to_string(),
            run,
            task_type: "test".to_string(),
            entity_type: "capture".to_string(),
            entity_id: 1,
            status_code: status.as_i32(),
            attempts: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // --- `latest_runs_per_task` (pure) ---

    #[test]
    fn collapse_keeps_the_latest_run() {
        let kept = latest_runs_per_task(vec![
            row("a", 1, TaskRunStatus::CompleteFailure),
            row("a", 2, TaskRunStatus::Queued),
        ]);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].run, 2, "the rerun supersedes the run before it");
    }

    #[test]
    fn collapse_returns_one_row_per_envelope() {
        let kept = latest_runs_per_task(vec![
            row("a", 1, TaskRunStatus::Queued),
            row("b", 1, TaskRunStatus::InProgress),
            row("a", 2, TaskRunStatus::Queued),
        ]);

        assert_eq!(kept.len(), 2, "one row per logical task");
    }

    #[test]
    fn collapse_is_independent_of_input_order() {
        let rows = vec![
            row("a", 3, TaskRunStatus::CompleteSuccess),
            row("b", 1, TaskRunStatus::Queued),
            row("a", 1, TaskRunStatus::CompleteFailure),
            row("a", 2, TaskRunStatus::InProgress),
        ];

        let kept = latest_runs_per_task(rows);

        assert_eq!(kept.len(), 2);
        assert_eq!(
            kept.iter()
                .find(|item| item.envelope_id == "a")
                .unwrap()
                .run,
            3
        );
    }

    #[test]
    fn collapse_preserves_latest_row_status_and_attempts() {
        let mut latest = row("a", 2, TaskRunStatus::InProgress);
        latest.attempts = 7;

        let kept = latest_runs_per_task(vec![row("a", 1, TaskRunStatus::CompleteFailure), latest]);

        assert_eq!(kept[0].status_code, TaskRunStatus::InProgress.as_i32());
        assert_eq!(kept[0].attempts, 7);
    }

    /// The whole point of dropping the incomplete filter: a caller must be able
    /// to observe that its work finished.
    #[test]
    fn collapse_keeps_a_completed_latest_run() {
        let kept = latest_runs_per_task(vec![row("a", 1, TaskRunStatus::CompleteSuccess)]);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].status_code, TaskRunStatus::CompleteSuccess.as_i32());
    }

    #[test]
    fn collapse_keeps_every_task_regardless_of_status() {
        let kept = latest_runs_per_task(vec![
            row("a", 1, TaskRunStatus::CompleteSuccess),
            row("b", 1, TaskRunStatus::CompleteFailure),
            row("c", 1, TaskRunStatus::Queued),
        ]);

        assert_eq!(kept.len(), 3, "no status is filtered out");
    }

    // --- DB-backed ---

    #[tokio::test]
    async fn create_run_persists_the_full_identity() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = envelope(1, 42, 1);

        assert!(
            tracker
                .create_run(&env)
                .await
                .expect("create_run should succeed")
        );

        let stored = tracker
            .query_latest_run(&env.envelope_id)
            .await
            .expect("query should succeed")
            .expect("row should exist");

        assert_eq!(stored.run, 1);
        assert_eq!(stored.user_id, 1);
        assert_eq!(stored.entity_id, 42);
        assert_eq!(stored.entity_type, "capture");
        assert_eq!(stored.task_type, "test");
        assert_eq!(stored.status_code, TaskRunStatus::Queued.as_i32());
        assert_eq!(stored.attempts, 0);
    }

    #[tokio::test]
    async fn create_run_always_starts_queued_with_zero_attempts() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));
        let task_envelope = envelope(1, 52, 1);

        assert!(tracker.create_run(&task_envelope).await.unwrap());

        let stored = tracker
            .query_run_status(&task_envelope.envelope_id, task_envelope.run)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status_code, TaskRunStatus::Queued.as_i32());
        assert_eq!(stored.attempts, 0);

        let published = notifier.0.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].payload.status, TaskRunStatus::Queued);
        assert_eq!(published[0].payload.attempts, 0);
    }

    #[tokio::test]
    async fn successful_status_writes_publish_events_but_conflicts_and_missing_updates_do_not() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let notifier = RecordingNotifier::default();
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(notifier.clone())));
        let task_envelope = envelope(1, 42, 1);

        assert!(tracker.create_run(&task_envelope).await.unwrap());
        assert!(!tracker.create_run(&task_envelope).await.unwrap());
        tracker
            .update_run(&task_envelope, TaskRunStatus::InProgress, 1)
            .await
            .unwrap();
        tracker
            .update_run(&envelope(1, 99, 1), TaskRunStatus::CompleteSuccess, 1)
            .await
            .unwrap();

        let events = notifier.0.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload.status, TaskRunStatus::Queued);
        assert_eq!(events[1].payload.status, TaskRunStatus::InProgress);
        assert_eq!(events[1].entity_id, 42);
    }

    #[tokio::test]
    async fn notification_failure_does_not_fail_a_status_write() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), Some(Arc::new(FailingNotifier)));
        let task_envelope = envelope(1, 42, 1);

        assert!(
            tracker
                .create_run(&task_envelope)
                .await
                .expect("best-effort notification failure must not fail persistence")
        );
        tracker
            .update_run(&task_envelope, TaskRunStatus::InProgress, 1)
            .await
            .expect("best-effort notification failure must not fail persistence");

        let stored = tracker
            .query_run_status(&task_envelope.envelope_id, task_envelope.run)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status_code, TaskRunStatus::InProgress.as_i32());
    }

    /// The unique index is what actually prevents a duplicate submission, so the
    /// tracker must report the conflict rather than propagate it as an error.
    #[tokio::test]
    async fn create_run_refuses_a_duplicate_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = envelope(1, 42, 1);

        let first = tracker
            .create_run(&env)
            .await
            .expect("first create_run should succeed");
        let second = tracker
            .create_run(&env)
            .await
            .expect("a conflict must not be an error");

        assert!(first, "the first insert wins");
        assert!(!second, "the second loses the race");
    }

    #[tokio::test]
    async fn concurrent_create_run_calls_are_arbitrated_by_unique_index() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let first = envelope(1, 42, 1);
        let second = envelope(1, 42, 1);

        let (first_created, second_created) =
            tokio::join!(tracker.create_run(&first), tracker.create_run(&second),);
        let created = [first_created.unwrap(), second_created.unwrap()];

        assert_eq!(
            created.iter().filter(|was_created| **was_created).count(),
            1,
            "the unique (envelope_id, run) constraint must arbitrate concurrent inserts"
        );
    }

    #[tokio::test]
    async fn create_run_allows_a_new_run_of_the_same_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        assert!(
            create_run_with_status(
                &tracker,
                &envelope(1, 42, 1),
                TaskRunStatus::CompleteSuccess,
                1,
            )
            .await
            .expect("run 1 should be created")
        );
        assert!(
            tracker
                .create_run(&envelope(1, 42, 2))
                .await
                .expect("run 2 should be created")
        );

        let latest = tracker
            .query_latest_run(&envelope(1, 42, 1).envelope_id)
            .await
            .expect("query should succeed")
            .expect("a row should exist");

        assert_eq!(latest.run, 2, "latest_run picks the highest run");
    }

    #[tokio::test]
    async fn create_run_accepts_the_required_payload() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let env = envelope(1, 42, 1);
        let result = tracker.create_run(&env).await;

        assert!(result.is_ok(), "a valid envelope carries its task payload");
    }

    #[tokio::test]
    async fn update_run_touches_only_its_own_run() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let run1 = envelope(1, 42, 1);
        let run2 = envelope(1, 42, 2);

        create_run_with_status(&tracker, &run1, TaskRunStatus::CompleteFailure, 1)
            .await
            .expect("run 1 should be created");
        tracker
            .create_run(&run2)
            .await
            .expect("run 2 should be created");

        tracker
            .update_run(&run1, TaskRunStatus::InProgress, 7)
            .await
            .expect("update should succeed");

        let stored1 = tracker
            .query_run_status(&run1.envelope_id, 1)
            .await
            .expect("query should succeed")
            .expect("run 1 should exist");
        let stored2 = tracker
            .query_run_status(&run2.envelope_id, 2)
            .await
            .expect("query should succeed")
            .expect("run 2 should exist");

        assert_eq!(stored1.status_code, TaskRunStatus::InProgress.as_i32());
        assert_eq!(stored1.attempts, 7);
        assert_eq!(
            stored2.status_code,
            TaskRunStatus::Queued.as_i32(),
            "run 2 must be untouched"
        );

        // A run that was never written is simply absent, not an error.
        let missing = tracker
            .query_run_status(&run1.envelope_id, 99)
            .await
            .expect("query should succeed");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn query_latest_status_for_entities_is_user_scoped() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        tracker
            .create_run(&envelope(1, 42, 1))
            .await
            .expect("create_run should succeed");

        let mine = tracker
            .query_latest_status_for_entities(1, "capture", &[42])
            .await
            .expect("query should succeed");
        let theirs = tracker
            .query_latest_status_for_entities(2, "capture", &[42])
            .await
            .expect("query should succeed");

        assert_eq!(mine.len(), 1);
        assert!(theirs.is_empty(), "entity ids are not a security boundary");
    }

    #[tokio::test]
    async fn query_latest_status_for_entities_filters_and_collapses_runs() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        create_run_with_status(
            &tracker,
            &envelope(1, 42, 1),
            TaskRunStatus::CompleteFailure,
            2,
        )
        .await
        .expect("older run should be created");
        create_run_with_status(
            &tracker,
            &envelope(1, 42, 2),
            TaskRunStatus::CompleteSuccess,
            1,
        )
        .await
        .expect("newer run should be created");
        tracker
            .create_run(&envelope(1, 43, 1))
            .await
            .expect("second entity should be created");
        create_run_with_status(&tracker, &envelope(2, 42, 1), TaskRunStatus::InProgress, 1)
            .await
            .expect("other user's entity should be created");

        let rows = tracker
            .query_latest_status_for_entities(1, "capture", &[42, 43, 999])
            .await
            .expect("multi-entity query should succeed");

        assert_eq!(rows.len(), 2, "only matching user/entity rows are returned");
        assert_eq!(
            rows.iter().find(|row| row.entity_id == 42).unwrap().run,
            2,
            "only the latest run for an entity is returned"
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.entity_id == 42)
                .unwrap()
                .status_code,
            TaskRunStatus::CompleteSuccess.as_i32()
        );
    }

    #[tokio::test]
    async fn query_latest_status_for_entities_empty_input_returns_empty() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        let rows = tracker
            .query_latest_status_for_entities(1, "capture", &[])
            .await
            .expect("empty query should succeed");

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn query_latest_status_for_entities_keeps_distinct_tasks_and_filters_entity_type() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        create_run_with_status(
            &tracker,
            &envelope(1, 42, 1),
            TaskRunStatus::CompleteSuccess,
            1,
        )
        .await
        .unwrap();
        let search_task = TaskEnvelope::new(
            1,
            crate::logic::search_index::SearchIndexTask { capture_id: 42 },
            1,
        );
        tracker.create_run(&search_task).await.unwrap();
        let spark_task = TaskEnvelope::new(
            1,
            crate::logic::spark::SparkTask {
                spark_id: 42,
                capture_ids: vec![1],
            },
            1,
        );
        create_run_with_status(&tracker, &spark_task, TaskRunStatus::InProgress, 1)
            .await
            .unwrap();

        let capture_rows = tracker
            .query_latest_status_for_entities(1, "capture", &[42])
            .await
            .unwrap();
        assert_eq!(
            capture_rows.len(),
            2,
            "different logical tasks remain distinct"
        );
        assert!(capture_rows.iter().all(|row| row.entity_type == "capture"));
        assert!(capture_rows.iter().any(|row| row.task_type == "test"));
        assert!(
            capture_rows
                .iter()
                .any(|row| row.task_type == "search_index")
        );

        let spark_rows = tracker
            .query_latest_status_for_entities(1, "spark", &[42])
            .await
            .unwrap();
        assert_eq!(spark_rows.len(), 1);
        assert_eq!(spark_rows[0].task_type, "spark");
    }

    #[tokio::test]
    async fn latest_run_returns_highest_run_and_none_for_missing_task() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);
        let first = envelope(1, 42, 1);
        let second = envelope(1, 42, 2);

        create_run_with_status(&tracker, &first, TaskRunStatus::CompleteFailure, 1)
            .await
            .unwrap();
        tracker.create_run(&second).await.unwrap();

        assert_eq!(
            tracker
                .query_latest_run(&first.envelope_id)
                .await
                .unwrap()
                .unwrap()
                .run,
            2
        );
        assert!(
            tracker
                .query_latest_run("missing-envelope")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn update_run_for_missing_row_is_a_noop() {
        let Some(db) = crate::test_support::test_db::test_db().await else {
            return;
        };
        let tracker = TaskRunTracker::new(db.handle(), None);

        tracker
            .update_run(&envelope(1, 42, 1), TaskRunStatus::InProgress, 1)
            .await
            .expect("updating a missing run is a no-op");

        assert!(
            tracker
                .query_latest_run("u1-test-capture42")
                .await
                .unwrap()
                .is_none()
        );
    }
}
